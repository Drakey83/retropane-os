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

mod pen;
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

pub mod doc;
pub mod export;
pub mod import;
pub mod words;

use doc::INK_LINE;

/// A stroke's style bit: an extension byte follows its first point (the
/// ink block it belongs to, and whether a width for each point follows its
/// steps). Strokes written before blocks have none: block 0, one width.
pub const EXT: u8 = 0x80;
/// The extension byte's bit: the stroke's widths follow its steps, four
/// bits a point ([`width_scale`]).
const EXT_WIDTHS: u8 = 0x80;

/// The packed stroke `ink` starts with: its length in bytes (`None`
/// where the ink ends, or breaks).
fn stroke_len(ink: &[u8]) -> Option<usize> {
    let count = usize::from(*ink.first()?);
    let style = *ink.get(1)?;
    let mut len = 6 + 2 * count.checked_sub(1)?;
    if style & EXT != 0 {
        let ext = *ink.get(6)?;
        len += 1;
        if ext & EXT_WIDTHS != 0 {
            len += count.div_ceil(2);
        }
    }
    (len <= ink.len()).then_some(len)
}

/// A packed stroke unpacked: its style (without [`EXT`]), its points'
/// count, its ink block, and whether `widths` were filled.
#[derive(Clone, Copy, Debug)]
struct Unpacked {
    style: u8,
    n: usize,
    block: u8,
    widths: bool,
}

/// A packed stroke ([`stroke_len`]'s bytes) as its points (and, where it
/// has them, its widths).
fn unpack_all(s: &[u8], pts: &mut [(i16, i16); 255], widths: &mut [u8; 255]) -> Unpacked {
    let count = usize::from(s[0]);
    let (ext, steps) = if s[1] & EXT != 0 { (s[6], 7) } else { (0, 6) };
    let mut x = i16::from_le_bytes([s[2], s[3]]);
    let mut y = i16::from_le_bytes([s[4], s[5]]);
    pts[0] = (x, y);
    for k in 1..count {
        x = x.saturating_add(i16::from(s[steps + 2 * k - 2] as i8));
        y = y.saturating_add(i16::from(s[steps + 2 * k - 1] as i8));
        pts[k] = (x, y);
    }
    let has = ext & EXT_WIDTHS != 0;
    if has {
        let w = steps + 2 * (count - 1);
        for (k, out) in widths.iter_mut().enumerate().take(count) {
            let b = s[w + k / 2];
            *out = if k % 2 == 0 { b & 15 } else { b >> 4 };
        }
    }
    Unpacked {
        style: s[1] & !EXT,
        n: count,
        block: ext & 0x7F,
        widths: has,
    }
}

/// [`unpack_all`] for the points alone: its style and how many.
fn unpack(s: &[u8], pts: &mut [(i16, i16); 255]) -> (u8, usize) {
    let mut w = [0u8; 255];
    let u = unpack_all(s, pts, &mut w);
    (u.style, u.n)
}

/// The block a packed stroke belongs to.
fn block_of(s: &[u8]) -> u8 {
    if s[1] & EXT != 0 { s[6] & 0x7F } else { 0 }
}

/// A point's width (four bits) as a scale of the stroke's size, in 1/12:
/// 8 is the size itself, 0 a third of it, 15 nearly twice.
#[must_use]
pub const fn width_scale(w: u8) -> i32 {
    (w as i32 & 15) + 4
}

/// How much of `ink` is whole strokes that fit a field (the tests': a
/// field never ends mid-stroke).
#[cfg(test)]
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
        if r.num(tag::PARENT) == Some(id) && n < PAGES && !doc::is_text_page(&r) {
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

/// The first stroke of ink block `block` a point of the pen's `path` came
/// within 8 px of taken out of `ink`: its bytes closed over.
#[inline(never)]
fn erase_in(ink: &mut [u8], inn: &mut usize, path: &[(i16, i16)], block: u8) -> bool {
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
        if block_of(&ink[at..at + len]) == block && near(&pts[..k]) {
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

/// The ink a note's own record takes: its text may always be 2,000 bytes
/// beside it (the record holds about 4,080 with its title and marks).
/// The first pages filled the note's record with 4,000 bytes of ink, and
/// then nothing could be typed in it (the owner on the X51v, 2026-11-15:
/// "list doesnt seem to do anything in notes"). The ink past this is its
/// pages'.
const OWN_INK: usize = 1900;

/// A change to a record's ink.
enum InkOp<'a> {
    /// A packed stroke added at its end.
    Add(&'a [u8]),
    /// The first stroke of the block the pen's path crossed taken out.
    Erase(&'a [(i16, i16)], u8),
    /// Its last stroke taken off.
    Last,
}

/// One change to the ink of record `rec` (a note, or with `page` one of
/// its ink pages, removed once it is empty): false where it did not
/// apply (no room, no stroke crossed, no ink). The ink is two fields of
/// whole strokes, the second following the first, and a change copies
/// only the one it touches: a stroke is added to the second once that
/// has any, so their order holds. (The first version copied the text and
/// both fields for every change: a 6 to 7.5 KB frame on a 40 KB stack,
/// over the stack rule's 4,096.)
#[inline(never)]
fn ink_op(cx: &Ctx, rec: u32, op: InkOp<'_>, page: bool) -> bool {
    use rp_pim::store::TEXT_MAX;
    let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok()) else {
        return false;
    };
    let Some(store) = guard.as_mut() else {
        return false;
    };
    let mut buf = [0u8; TEXT_MAX];
    let lens = |store: &rp_pim::store::Store<'_>| {
        store
            .get(rec)
            .map(|r| (r.text(tag::INK).len(), r.text(tag::INK_MORE).len()))
    };
    let Some((a, b)) = lens(store) else {
        return false;
    };
    // The fields to try, in the order the change reads them.
    let fields: [u8; 2] = match op {
        // The note's own record: its first field only, to `OWN_INK`.
        InkOp::Add(s) if !page && (b > 0 || a + s.len() > OWN_INK) => return false,
        InkOp::Add(s) if b > 0 || a + s.len() > TEXT_MAX => [tag::INK_MORE, 0],
        InkOp::Add(_) => [tag::INK, 0],
        InkOp::Erase(..) => [tag::INK, tag::INK_MORE],
        InkOp::Last if b > 0 => [tag::INK_MORE, 0],
        InkOp::Last => [tag::INK, 0],
    };
    let mut done = false;
    for t in fields.into_iter().filter(|&t| t != 0) {
        let Some(r) = store.get(rec) else {
            return false;
        };
        let mut n = r.text(t).len().min(buf.len());
        buf[..n].copy_from_slice(&r.text(t)[..n]);
        let changed = match op {
            InkOp::Add(s) => buf.get_mut(n..n + s.len()).is_some_and(|to| {
                to.copy_from_slice(s);
                n += s.len();
                true
            }),
            InkOp::Erase(path, block) => erase_in(&mut buf, &mut n, path, block),
            InkOp::Last => drop_last(&buf, &mut n),
        };
        if changed {
            done = store.change(rec, &[(t, &buf[..n])]).is_ok();
            break;
        }
    }
    if done && page && lens(store) == Some((0, 0)) {
        let _ = store.remove(rec);
    }
    done
}

/// The packed stroke `stroke` added at the end of note `id`'s ink: the
/// last page's, or the note's own while it has none; where that is full, a
/// new page. (Its ink block's line is the caller's: the stroke names it.)
fn add_stroke(cx: &Ctx, id: u32, stroke: &[u8]) -> bool {
    let (page, np) = pages_of(cx, id);
    let on_last = match page[..np].last() {
        Some(&p) => ink_op(cx, p, InkOp::Add(stroke), true),
        None => ink_op(cx, id, InkOp::Add(stroke), false),
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
                            &[(tag::PARENT, &id.to_le_bytes()), (tag::INK, stroke)],
                        )
                        .ok()
                })
                .is_some())
}

/// An edit to note `id`'s text through the app's store
/// ([`doc::edit`]).
fn text_op(cx: &Ctx, id: u32, e: &doc::Edit<'_>) -> Result<([u8; 4], usize), doc::Refused> {
    let mut g = cx
        .pim
        .and_then(|c| c.try_borrow_mut().ok())
        .ok_or(doc::Refused::Full)?;
    let store = g.as_mut().ok_or(doc::Refused::Full)?;
    doc::edit(store, id, e)
}

/// The last stroke of record `rec`'s ink copied into `out`: its length (0
/// with no ink).
#[inline(never)]
fn last_stroke(cx: &Ctx, rec: u32, out: &mut [u8; STROKE_MAX]) -> usize {
    let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
        return 0;
    };
    let Some(r) = guard.as_ref().and_then(|s| s.get(rec)) else {
        return 0;
    };
    let field = if r.text(tag::INK_MORE).is_empty() {
        r.text(tag::INK)
    } else {
        r.text(tag::INK_MORE)
    };
    let (mut at, mut last) = (0, None);
    while let Some(len) = stroke_len(&field[at..]) {
        last = Some((at, len));
        at += len;
    }
    match last {
        Some((at, len)) if len <= out.len() => {
            out[..len].copy_from_slice(&field[at..at + len]);
            len
        }
        _ => 0,
    }
}

/// Where `text` holds the search `q`, whatever its capitals.
fn match_at(text: &[u8], q: &[u8]) -> Option<usize> {
    (!q.is_empty() && text.len() >= q.len())
        .then(|| {
            text.windows(q.len())
                .position(|w| w.eq_ignore_ascii_case(q))
        })
        .flatten()
}

/// `text` on a note's paper as [`kit::fitted`] draws it, the part that
/// holds the search `q` marked first as a highlighter marks paper (a
/// yellow band under the letters: a search's matches were found but not
/// shown).
fn marked(
    f: &mut Frame,
    (x, y): (u16, u16),
    font: &'static rp_app::Font,
    colour: Argb,
    text: &[u8],
    max: u16,
    q: &[u8],
) {
    if let Some(m) = match_at(text, q) {
        let x0 = font.width(&text[..m]);
        let w = font
            .width(&text[m..m + q.len()])
            .min(max.saturating_sub(x0));
        if w > 0 {
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(
                    x + x0,
                    y.saturating_sub(u16::from(font.ascent)),
                    w,
                    u16::from(font.line),
                ),
                color: kit::keep(0xFFFF_E066),
            });
        }
    }
    kit::fitted(f, x, y, font, colour, text, max);
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
/// A marker stroke's style bit: it takes the colour in bits 0-2 (the
/// owner on the X51v, 2026-11-15: a colour chosen with the marker
/// switched to the pen). Without it, the marker's own yellow (every
/// marker stroke written before).
pub const MARKER_HUE: u8 = 0x40;
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
fn simplify(pts: &mut [(i16, i16); LIVE], ws: &mut [u8; LIVE], n: usize) -> usize {
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
            ws[m] = ws[k];
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
    for (si, st) in strokes.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
        let style = styles.get(si).copied().unwrap_or(0) & !EXT;
        n += encode_stroke(st, None, style, 0, out.get_mut(n..)?)?;
    }
    Some(n)
}

/// One stroke packed (version 2): its points, a width for each where
/// given (four bits, [`width_scale`]), its style and its ink block (block
/// 0 with no widths packs as version 1, the way every stroke before blocks
/// was kept). Long steps are split (a split point takes the width before
/// it), so nothing is lost. The length, or `None` when it doesn't fit.
pub fn encode_stroke(
    st: &[(i16, i16)],
    widths: Option<&[u8]>,
    style: u8,
    block: u8,
    out: &mut [u8],
) -> Option<usize> {
    if st.is_empty() {
        return Some(0);
    }
    let mut pts = [(0i16, 0i16); 255];
    let mut ws = [8u8; 255];
    let w_of = |k: usize| widths.and_then(|w| w.get(k)).copied().unwrap_or(8) & 15;
    let mut k = 1;
    let mut last = st[0];
    pts[0] = last;
    ws[0] = w_of(0);
    for (i, &p) in st.iter().enumerate().skip(1) {
        while k < pts.len() && ((p.0 - last.0).abs() > 127 || (p.1 - last.1).abs() > 127) {
            last = (
                last.0 + (p.0 - last.0).clamp(-127, 127),
                last.1 + (p.1 - last.1).clamp(-127, 127),
            );
            pts[k] = last;
            ws[k] = w_of(i - 1);
            k += 1;
        }
        if k < pts.len() {
            pts[k] = p;
            ws[k] = w_of(i);
            k += 1;
            last = p;
        }
    }
    let ext = block != 0 || widths.is_some();
    let mut n = 0;
    let mut put = |b: u8| -> Option<()> {
        *out.get_mut(n)? = b;
        n += 1;
        Some(())
    };
    put(k as u8)?;
    put(if ext { style | EXT } else { style & !EXT })?;
    for b in pts[0].0.to_le_bytes().iter().chain(&pts[0].1.to_le_bytes()) {
        put(*b)?;
    }
    if ext {
        put((block & 0x7F) | if widths.is_some() { EXT_WIDTHS } else { 0 })?;
    }
    for w in pts[..k].windows(2) {
        put((w[1].0 - w[0].0) as i8 as u8)?;
        put((w[1].1 - w[0].1) as i8 as u8)?;
    }
    if widths.is_some() {
        for pair in ws[..k].chunks(2) {
            put(pair[0] | pair.get(1).copied().unwrap_or(0) << 4)?;
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
    let mut one = [(0i16, 0i16); 255];
    while ns < ends.len() {
        let Some(len) = stroke_len(&ink[at..]) else {
            break;
        };
        let (style, count) = unpack(&ink[at..at + len], &mut one);
        if np + count > pts.len() {
            break;
        }
        if let Some(st) = styles.get_mut(ns) {
            *st = style;
        }
        pts[np..np + count].copy_from_slice(&one[..count]);
        np += count;
        ends[ns] = np as u16;
        ns += 1;
        at += len;
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
    /// It holds strokes (its own ink field: the first pages hold them).
    inked: bool,
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
            inked: !r.text(tag::INK).is_empty(),
        };
        for l in r.text(tag::TEXT).split(|&c| c == b'\n') {
            if row.n == row.lines.len() {
                break;
            }
            let l = l.strip_prefix(&[doc::CONT]).unwrap_or(l);
            row.lines[row.n] = if doc::ink_block(l).is_some() {
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
        self.inked || self.lines[..self.n].iter().any(|l| matches!(l, Line::Ink))
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
    // Soft paper, not white: half of white's light (it was 84%, the
    // brightest thing in either look; the owner on the X51v, 2026-11-15,
    // "light mode still seems too blinding"). Ink still stands on it:
    // the black pen at 9:1.
    kit::keep(kit::mix(0xFFCA_C4B6, c, 0x50))
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
const TOOLS: [&[u8]; 7] = [
    b"PEN", b"MARKER", b"ERASE", b"TEXT", b"LIST", b"UNDO", b"REDO",
];
/// Tool key `k`: each as wide as its word needs, so seven fit the strip
/// (six of one width left no room for REDO).
const fn tool_key(k: usize) -> Rect {
    const W: [u16; 7] = [50, 78, 64, 56, 54, 60, 60];
    let mut x = STRIP.x + 5;
    let mut i = 0;
    while i < k && i < 6 {
        x += W[i] + 4;
        i += 1;
    }
    Rect::new(x, STRIP.y + 6, W[if k < 7 { k } else { 6 }], 36)
}
/// What UNDO took out, for REDO: each entry its bytes (a packed stroke,
/// or a typed character), then its length (two bytes) and kind.
const REDO_ROOM: usize = 1200;
/// A hold this many ticks (about 30 ms each: half a second) on a sheet of
/// the home offers Delete.
const HOLD_TICKS: u8 = 16;
const PAPER: Rect = Rect::new(8, 40, W - 16, 404);
/// The ink panel's size (the store's coordinates).
const INK_W: u16 = kit::INK_SHEET.0;
const INK_H: u16 = kit::INK_SHEET.1;
/// Where the sheet's ink lies: under the title, down to the tray.
const INK_AT: (u16, u16) = (PAPER.x + 8, PAPER.y + 56);
/// Points a stroke being written keeps.
const LIVE: usize = 200;

/// The Notes program.
pub struct Notes {
    rows: [Option<Row>; 24],
    /// The note's tool and pen colour; the stroke being written; where the
    /// ink panel lies on screen; the checklist rows on screen; typing.
    tool: Tool,
    pen: u8,
    /// The marker's colour (a well chosen while the marker is the tool):
    /// none, its own yellow.
    marker_hue: Option<u8>,
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
    checks: [(Rect, usize); 12],
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
    /// The home's long press: the sheet under the stylus and the ticks it
    /// has been held; the note (its place) Delete is offered for, its key,
    /// and whether the stylus that asked is still down.
    hold: Option<(u8, u8)>,
    ask: Option<u8>,
    ask_key: Rect,
    ask_held: bool,
    /// Typing was refused (the note's text is full): said in the tray.
    text_full: bool,
    /// What UNDO took out of the note `redo_of`, newest last
    /// ([`REDO_ROOM`]); writing anything new forgets it.
    redo: [u8; REDO_ROOM],
    redo_n: usize,
    redo_of: Option<u32>,
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
    /// The open note's ink: a coverage layer a colour in use (the colour,
    /// its picture), over the whole sheet.
    ink: [Option<(Argb, Pic)>; kit::INK_LAYERS as usize],
    /// The stylus went down on the sheet to write (not on a tick box).
    inking: bool,
    minis: [Option<(u32, Pic)>; 4],
    /// The miniatures this frame drew (a bit each).
    mini_used: u8,
    ring: Option<Pic>,
    full: Option<Pic>,
    painted: Option<(View, u32, u16, u16)>,
    /// The pen's filter, and the stroke being written's widths, its ink
    /// block and that block's top below the ink layers' (negative:
    /// scrolled above).
    filt: pen::Filter,
    live_w: [u8; LIVE],
    live_block: u8,
    live_dy: i32,
    /// The open note's layout: the rows scrolled past, the items on
    /// screen, every row it has, its lines, its last item (and that item's
    /// first row), the next ink block's number, and each block's rows.
    scroll: u16,
    vis: [Vis; 24],
    n_vis: usize,
    total_rows: u16,
    n_lines: usize,
    last: Option<(Item, u16)>,
    next_block: u8,
    rows_of: [u8; BLOCKS],
    /// The line typing goes into (none: a new line at the end), and the
    /// view follows it.
    cursor: Option<usize>,
    follow: bool,
    /// A stylus down off the ink: where (a drag scrolls, a tap places the
    /// caret or ticks).
    tap: Option<(u16, u16, bool)>,
    /// The block layout's design (the mock-ups' variants; 0 the default).
    pub design: u8,
    /// The menu's second rows: 1 the export formats, 2 the files to
    /// import.
    menu_row: u8,
    /// A file step waiting for the shell, and the one under way.
    asking: Option<rp_app::FileAsk>,
    job: Job,
    /// The files offered for import.
    files: [Text<64>; 8],
    n_files: usize,
    /// What the tray says after a file step.
    said: &'static [u8],
    reader: import::Reader,
    png: export::PngWork,
    /// The word suggestions, the words learned from the notes (and the
    /// store change they were counted at).
    sugg: [[u8; words::WORD_MAX]; 3],
    sugg_n: [usize; 3],
    learned: words::Learned,
    learned_at: Option<u32>,
    /// Whether the next letter typed is a capital.
    caps_next: bool,
}

/// What a line of an open note is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Text,
    Check(bool),
    Ink(u8),
}

/// An item of the open note on screen: its line, its first row (counted
/// from the note's top), its rows.
#[derive(Clone, Copy, Debug)]
struct Vis {
    line: usize,
    row: u16,
    rows: u16,
    item: Item,
}

/// A file step under way.
#[derive(Clone, Copy, Debug)]
enum Job {
    None,
    /// Note `id` written as `fmt` (`block` for ink; the blocks still to go
    /// after it), `len` bytes.
    Write {
        id: u32,
        fmt: export::Format,
        block: u8,
        left: u128,
    },
    /// The folder listed for import.
    List,
    /// A file read into note `id`: its lines put in so far.
    Read { id: u32, put: u32 },
}

/// The open note's first row's top (under its title) and a row's height.
const TOP: u16 = PAPER.y + 56;
const ROW: u16 = 28;
/// The scroll rail: a drag along the paper's right edge always scrolls.
const RAIL: u16 = 28;

/// Paints `strokes` (in the sheet's 448 x 352) into `px` (`w` x `h`, scaled to
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

/// The colour a stroke of `style` is drawn in, its alpha the stroke's
/// opacity: a pen's, the marker's own yellow, or the marker in a pen's
/// colour (both see-through).
fn stroke_colour(style: u8) -> Argb {
    let pen = PEN[usize::from(style & 7).min(5)];
    if style & MARKER == 0 {
        pen
    } else if style & MARKER_HUE != 0 {
        kit::keep((pen & 0x00FF_FFFF) | 0x7000_0000)
    } else {
        kit::keep(0x70FF_E040)
    }
}

/// The brush: `strokes` (in the sheet's 448 x 352) laid as stamps into a
/// `w` x `h` picture scaled to fit, each stamp's pixels given to `put`
/// (the pixel's index, its coverage 0-255 with the colour's opacity in
/// it, the colour): one copy of the curve and the stamp for the
/// miniatures' 32-bit pictures and the sheet's coverage layers.
fn brush(
    w: u16,
    h: u16,
    strokes: &[&[(i16, i16)]],
    styles: &[u8],
    only: Option<Argb>,
    put: &mut dyn FnMut(usize, u32, Argb),
) {
    let sx = i32::from(w) * 256 / i32::from(INK_W);
    let sy = i32::from(h) * 256 / i32::from(INK_H);
    brush_at(w, h, sx.min(sy), strokes, styles, &[], (0, 0), only, put);
}

/// [`brush`] with each stroke's point widths (empty: one width) and the
/// strokes moved by `off` (sheet pixels: an ink block's place on the
/// sheet): the radius goes from one point's width to the next along the
/// curve, and a segment wholly off the picture is skipped.
#[allow(clippy::too_many_arguments)]
fn brush_at(
    w: u16,
    h: u16,
    s: i32,
    strokes: &[&[(i16, i16)]],
    styles: &[u8],
    widths: &[&[u8]],
    off: (i32, i32),
    only: Option<Argb>,
    put: &mut dyn FnMut(usize, u32, Argb),
) {
    // Stamp radius in 1/16 px (at least a pixel's worth).
    let base_rad = (20 * s / 256).max(10);
    let mut stamp = |cx16: i32, cy16: i32, rad: i32, c: Argb| {
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
                    put((y * i32::from(w) + x) as usize, a, c);
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
        let c = only.unwrap_or(stroke_colour(style));
        let rad = match style & (SIZE_FINE | SIZE_BOLD) {
            SIZE_FINE => base_rad * 5 / 8,
            SIZE_BOLD => base_rad * 2,
            _ => base_rad,
        };
        let rad = if marker { rad * 3 } else { rad };
        let ws: &[u8] = widths.get(si).copied().unwrap_or(&[]);
        let wk = |k: isize| {
            let k = k.clamp(0, st.len() as isize - 1) as usize;
            ws.get(k).map_or(12, |&w| width_scale(w))
        };
        let p = |i: isize| {
            let k = i.clamp(0, st.len() as isize - 1) as usize;
            (
                (i32::from(st[k].0) + off.0) * s / 16,
                (i32::from(st[k].1) + off.1) * s / 16,
            )
        };
        let (wpx, hpx) = (i32::from(w) * 16, i32::from(h) * 16);
        // (A stroke of one point is a dot.)
        for i in 0..(st.len() as isize - 1).max(1) {
            let (p0, p1, p2, p3) = (p(i - 1), p(i), p(i + 1), p(i + 2));
            // A segment wholly off the picture (an ink block scrolled
            // past) costs nothing: the curve stays within its four points.
            let reach = rad * 2;
            let (lo_x, hi_x) = (p0.0.min(p1.0).min(p2.0).min(p3.0), p0.0.max(p1.0).max(p2.0).max(p3.0));
            let (lo_y, hi_y) = (p0.1.min(p1.1).min(p2.1).min(p3.1), p0.1.max(p1.1).max(p2.1).max(p3.1));
            if hi_x + reach < 0 || lo_x - reach > wpx || hi_y + reach < 0 || lo_y - reach > hpx {
                continue;
            }
            let (w1, w2) = (wk(i), wk(i + 1));
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
                // The width from this point's to the next's.
                let r = if marker {
                    rad
                } else {
                    (rad * (w1 * (256 - t) + w2 * t) / (256 * 12)).max(8)
                };
                stamp(cr(p0.0, p1.0, p2.0, p3.0), cr(p0.1, p1.1, p2.1, p3.1), r, c);
            }
        }
    }
}

/// [`paint_styled`] over what is there, into a 32-bit picture (a card's
/// miniature), the brush keeping the most coverage.
pub fn paint_over(
    px: &mut [Argb],
    w: u16,
    h: u16,
    strokes: &[&[(i16, i16)]],
    styles: &[u8],
    only: Option<Argb>,
) {
    brush(w, h, strokes, styles, only, &mut |i, a, c| {
        if let Some(p) = px.get_mut(i)
            && a > *p >> 24
        {
            *p = (c & 0x00FF_FFFF) | (a << 24);
        }
    });
}

/// The strokes of colour `want` among `strokes` painted over what is
/// there in a sheet's coverage layer (`Cmd::Cover`'s words: four bits a
/// pixel, the first pixel in the high half of the first byte), the brush
/// keeping the most coverage: the stroke being written a segment at a
/// time and the kept ink alike, so what is written is what is kept.
pub fn paint_cover(words: &mut [u32], strokes: &[&[(i16, i16)]], styles: &[u8], want: Argb) {
    paint_cover_at(words, strokes, styles, &[], 0, want);
}

/// [`paint_cover`] with each stroke's widths, its ink block `dy` pixels
/// below the layer's top (above it where negative: scrolled).
pub fn paint_cover_at(
    words: &mut [u32],
    strokes: &[&[(i16, i16)]],
    styles: &[u8],
    widths: &[&[u8]],
    dy: i32,
    want: Argb,
) {
    brush_at(INK_W, INK_H, 256, strokes, styles, widths, (0, dy), None, &mut |i, a, c| {
        if c != want {
            return;
        }
        let n = ((a + 8) / 17).min(15);
        let shift = 8 * ((i / 2) % 4) as u32 + if i.is_multiple_of(2) { 4 } else { 0 };
        if let Some(word) = words.get_mut(i / 8)
            && n > (*word >> shift) & 15
        {
            *word = (*word & !(15 << shift)) | (n << shift);
        }
    });
}

/// Every packed stroke of note `id` in the order written (the note's own
/// fields, then its ink pages'), given to `f`.
pub(crate) fn each_stroke(store: &rp_pim::store::Store<'_>, id: u32, f: &mut dyn FnMut(&[u8])) {
    let Some(rec) = store.get(id) else { return };
    let (page, np) = pages(store, id);
    let recs = core::iter::once(rec).chain(page[..np].iter().filter_map(|&i| store.get(i)));
    for field in recs.flat_map(|r| [r.text(tag::INK), r.text(tag::INK_MORE)]) {
        let mut at = 0;
        while let Some(n) = stroke_len(&field[at..]) {
            f(&field[at..at + n]);
            at += n;
        }
    }
}

/// The ink blocks a note can have (block numbers 0-94: [`doc::ink_line`]).
pub const BLOCKS: usize = 95;
/// A new ink block's height in ruled rows (196 px, the first panel's).
pub const BLOCK_ROWS: u8 = 7;

/// Each ink block's height in ruled rows (28 px): its lowest stroke and a
/// row and a half below it, [`BLOCK_ROWS`] at least, so a block grows as
/// it is written lower and never cuts a stroke.
#[must_use]
pub(crate) fn block_rows(store: &rp_pim::store::Store<'_>, id: u32) -> [u8; BLOCKS] {
    let mut low = [0i32; BLOCKS];
    let mut pts = [(0i16, 0i16); 255];
    each_stroke(store, id, &mut |p| {
        let (_, n) = unpack(p, &mut pts);
        if let Some(l) = low.get_mut(usize::from(block_of(p))) {
            for q in &pts[..n] {
                *l = (*l).max(i32::from(q.1));
            }
        }
    });
    let mut rows = [BLOCK_ROWS; BLOCKS];
    for (r, l) in rows.iter_mut().zip(low) {
        *r = ((l + 40 + 27) / 28).clamp(i32::from(BLOCK_ROWS), 255) as u8;
    }
    rows
}

/// Paints the ink of note `id` from the store into a new picture of
/// `w` x `h` in `c`: its blocks one under another, as the note lays them.
fn ink_pic(cx: &Ctx, id: u32, w: u16, h: u16, c: Argb) -> Option<Pic> {
    let pics = cx.pics?;
    let guard = cx.pim?.try_borrow().ok()?;
    let store = guard.as_ref()?;
    let rows = block_rows(store, id);
    // Each block's top: the blocks before it, in number order.
    let mut top = [0i32; BLOCKS];
    let used = export::inked_blocks(store, id);
    let mut y = 0;
    for (b, t) in top.iter_mut().enumerate() {
        *t = y;
        if used & (1 << b) != 0 {
            y += i32::from(rows[b]) * 28;
        }
    }
    let p = pics.alloc(w, h)?;
    pics.write(p, |px| {
        px.iter_mut().for_each(|q| *q = 0);
        let mut pts = [(0i16, 0i16); 255];
        let mut ws = [8u8; 255];
        each_stroke(store, id, &mut |packed| {
            let u = unpack_all(packed, &mut pts, &mut ws);
            let off = (0, top[usize::from(u.block).min(BLOCKS - 1)]);
            let widths: &[u8] = if u.widths { &ws[..u.n] } else { &[] };
            let s = (i32::from(w) * 256 / i32::from(INK_W)).min(i32::from(h) * 256 / i32::from(INK_H));
            brush_at(w, h, s, &[&pts[..u.n]], &[u.style], &[widths], off, Some(c), &mut |i, a, c| {
                if let Some(q) = px.get_mut(i)
                    && a > *q >> 24
                {
                    *q = (c & 0x00FF_FFFF) | (a << 24);
                }
            });
        });
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
            marker_hue: None,
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
            hold: None,
            ask: None,
            ask_key: Rect::new(0, 0, 0, 0),
            ask_held: false,
            text_full: false,
            redo: [0; REDO_ROOM],
            redo_n: 0,
            redo_of: None,
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
            ink: [None; kit::INK_LAYERS as usize],
            inking: false,
            minis: [None; 4],
            mini_used: 0,
            ring: None,
            full: None,
            painted: None,
            filt: pen::Filter::default(),
            live_w: [8; LIVE],
            live_block: 0,
            live_dy: 0,
            scroll: 0,
            vis: [Vis {
                line: 0,
                row: 0,
                rows: 0,
                item: Item::Text,
            }; 24],
            n_vis: 0,
            total_rows: 0,
            n_lines: 0,
            last: None,
            next_block: 1,
            rows_of: [BLOCK_ROWS; BLOCKS],
            cursor: None,
            follow: false,
            tap: None,
            design: 0,
            menu_row: 0,
            asking: None,
            job: Job::None,
            files: [Text::EMPTY; 8],
            n_files: 0,
            said: b"",
            reader: import::Reader::new(),
            png: export::PngWork::new(),
            sugg: [[0; words::WORD_MAX]; 3],
            sugg_n: [0; 3],
            learned: words::Learned::new(),
            learned_at: None,
            caps_next: true,
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
            for l in &mut self.ink {
                if let Some((_, pic)) = l.take() {
                    pics.free(pic);
                }
            }
        }
    }

    /// The layer for ink of `colour`: the one there is, or a new empty one
    /// (a layer for every colour ink can be: never short).
    fn layer_for(&mut self, pics: &rp_app::Pics, colour: Argb) -> Option<usize> {
        if let Some(k) = self
            .ink
            .iter()
            .position(|l| l.is_some_and(|l| l.0 == colour))
        {
            return Some(k);
        }
        let k = self.ink.iter().position(Option::is_none)?;
        let p = pics.alloc(kit::INK_LAYER.0, kit::INK_LAYER.1)?;
        pics.write(p, |w| w.iter_mut().for_each(|q| *q = 0));
        self.ink[k] = Some((colour, p));
        Some(k)
    }

    /// Note `id`'s ink painted from the store into its layers: the strokes
    /// of each ink block on screen, at the block's place (the note's own
    /// fields, then its pages').
    fn ink_layers(&mut self, cx: &Ctx, id: u32) {
        let Some(pics) = cx.pics else { return };
        for l in &mut self.ink {
            if let Some((_, p)) = l.take() {
                pics.free(p);
            }
        }
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return;
        };
        let Some(store) = guard.as_ref() else { return };
        // Each block's top below the layers' (none: not on screen).
        let mut dy = [None::<i32>; BLOCKS];
        for v in &self.vis[..self.n_vis] {
            if let Item::Ink(b) = v.item {
                dy[usize::from(b)] = Some(i32::from(ROW) * (i32::from(v.row) - i32::from(self.scroll)));
            }
        }
        let mut pts = [(0i16, 0i16); 255];
        let mut ws = [8u8; 255];
        each_stroke(store, id, &mut |packed| {
            let u = unpack_all(packed, &mut pts, &mut ws);
            let Some(d) = dy.get(usize::from(u.block)).copied().flatten() else {
                return;
            };
            let colour = stroke_colour(u.style);
            let widths: &[u8] = if u.widths { &ws[..u.n] } else { &[] };
            if let Some(l) = self.layer_for(pics, colour)
                && let Some((_, p)) = self.ink[l]
            {
                pics.write(p, |w| paint_cover_at(w, &[&pts[..u.n]], &[u.style], &[widths], d, colour));
            }
        });
    }

    /// The search typed: each note found when its title or text holds it
    /// (every chunk of its text).
    fn find(&mut self, cx: &Ctx) {
        let q = &self.query[..self.query_n];
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        for row in self.rows.iter_mut().flatten() {
            row.found = q.is_empty()
                || store.get(row.id).is_some_and(|r| {
                    let t = r.text(tag::TITLE);
                    t.len() >= q.len() && t.windows(q.len()).any(|w| w.eq_ignore_ascii_case(q))
                })
                || doc::holds(store, row.id, q);
        }
    }

    fn paint(&mut self, cx: &Ctx) {
        let rows = self.view_rows();
        let key = (self.view, self.seen.unwrap_or(0), self.scroll, rows);
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
                if let Some(row) = self.rows.get(usize::from(i)).copied().flatten() {
                    self.ink_layers(cx, row.id);
                }
            }
        }
        self.painted = Some(key);
    }

    /// The rows the open note shows (fewer while the keyboard is up).
    fn view_rows(&self) -> u16 {
        let bottom = if self.typing { 392 } else { TRAY.y };
        (bottom - TOP) / ROW
    }

    /// The open note laid out: its lines walked, each item's rows (a
    /// paragraph wrapped to the paper, a checklist item one, an ink block
    /// its rows), those on screen kept; the view moved to the caret's line
    /// when it follows it.
    fn layout(&mut self, cx: &Ctx, id: u32) {
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return;
        };
        let Some(store) = guard.as_ref() else { return };
        self.rows_of = block_rows(store, id);
        for _pass in 0..2 {
            let view = self.view_rows();
            let (top, rows_of, font) = (self.scroll, self.rows_of, cx.fonts.ui);
            let (mut vis, mut nv) = (self.vis, 0usize);
            let (mut row, mut last, mut next_block) = (0u16, None, 1u8);
            let mut cursor_at = None;
            let cursor = self.cursor;
            let n_lines = doc::for_lines(store, id, &mut |k, l| {
                let (item, rows) = match doc::ink_block(l) {
                    Some(b) => {
                        next_block = next_block.max(b + 1);
                        (Item::Ink(b), u16::from(rows_of[usize::from(b).min(BLOCKS - 1)]))
                    }
                    None if l.starts_with(b"[ ] ") => (Item::Check(false), 1),
                    None if l.starts_with(b"[x] ") => (Item::Check(true), 1),
                    None => (Item::Text, wrap(font, l, PAPER.w - 48, &mut |_| {})),
                };
                if cursor == Some(k) {
                    cursor_at = Some((row, rows));
                }
                if row + rows > top && row < top + view && nv < vis.len() {
                    vis[nv] = Vis { line: k, row, rows, item };
                    nv += 1;
                }
                last = Some((item, row));
                row = row.saturating_add(rows);
                true
            });
            (self.vis, self.n_vis, self.total_rows, self.n_lines) = (vis, nv, row, n_lines);
            self.last = last;
            self.next_block = next_block.min(94);
            // The caret's line in view (its last row at the foot).
            let want = match (self.follow, cursor_at) {
                (true, Some((r, n))) if r + n > top + view => Some(r + n - view.min(r + n)),
                (true, Some((r, _))) if r < top => Some(r),
                (true, None) if self.cursor.is_none() && row + 1 > top + view => Some(row + 1 - view),
                _ => None,
            };
            self.follow = false;
            match want {
                Some(s) if s != self.scroll => self.scroll = s,
                _ => break,
            }
        }
    }

    /// The item at screen row `y` (none: blank paper below the last).
    fn item_at(&self, y: u16) -> Option<Vis> {
        let r = self.scroll + (y.saturating_sub(TOP)) / ROW;
        self.vis[..self.n_vis]
            .iter()
            .copied()
            .find(|v| r >= v.row && r < v.row + v.rows)
    }

    /// A note's item's top on screen (above the paper's top where it is
    /// scrolled part away).
    fn screen_y(&self, row: u16) -> i32 {
        i32::from(TOP) + i32::from(ROW) * (i32::from(row) - i32::from(self.scroll))
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
        (x, mut y): (u16, u16),
        max: u16,
        most: usize,
        q: &[u8],
    ) -> u16 {
        let lines = note.lines[..note.n]
            .iter()
            .filter(|l| !matches!(l, Line::Ink))
            .take(most);
        for l in lines {
            match l {
                Line::Text(t) => marked(f, (x, y), cx.fonts.ui, INK_DIM, t.get(), max, q),
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
                    marked(f, (x + 20, y), cx.fonts.ui, col, t.get(), max - 20, q);
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
        // (A copy: the search typed is read while the miniatures are made.)
        let (query, qn) = (self.query, self.query_n);
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
            let sheet = kit::keep(0xFFC8_C0AA);
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
            marked(
                f,
                (r.x + 24, r.y + 52),
                cx.fonts.title,
                INK,
                title,
                r.w - 48,
                &query[..qn],
            );
            let ink = note.has_ink();
            Self::paper_lines(
                f,
                cx,
                &note,
                (r.x + 24, r.y + 82),
                if ink { 196 } else { r.w - 48 },
                3,
                &query[..qn],
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
            marked(
                f,
                (r.x + 12, r.y + 26),
                cx.fonts.ui,
                INK,
                title,
                r.w - 24 - pin,
                &query[..qn],
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
                (r.x + 12, r.y + 50),
                r.w - 24,
                usize::from(lines),
                &query[..qn],
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
        // Delete, offered by a long press, on the sheet itself.
        if let Some(place) = self.ask {
            match self.cards[..self.n_cards].iter().find(|c| c.1 == place) {
                Some(&(r, _)) => {
                    let key = Rect::new(r.x + 10, r.y + r.h / 2 - 20, r.w.min(220) - 20, 40);
                    kit::key(f, cx, key, kit::KeyState::Down);
                    kit::centred(
                        f,
                        key,
                        key.y + 26,
                        cx.fonts.ui,
                        kit::LATE,
                        b"Delete this note",
                    );
                    self.ask_key = key;
                }
                None => self.ask = None,
            }
        }
        kit::foot_tabs(f, cx, &FILTERS, usize::from(self.filter), tint::NOTES);
    }

    fn note(&mut self, f: &mut Frame, cx: &Ctx, i: u8) {
        self.n_checks = 0;
        self.ink_at = Some(INK_AT);
        let Some(note) = self.rows.get(usize::from(i)).copied().flatten() else {
            return;
        };
        // The paper: warm, ruled every 28 px.
        let paper = PAPER;
        f.push(rp_app::Cmd::Fill {
            r: paper,
            color: kit::keep(0xF0CE_C8B6),
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
        self.body(f, cx, note.id, note.colour);
        // The ink over the lines: the markers' layers first, the pens'
        // over them (a highlighter lies under the writing).
        let (ix, iy) = INK_AT;
        for see_through in [true, false] {
            for &(colour, pic) in self.ink.iter().flatten() {
                if (colour >> 24 < 0xF0) == see_through {
                    f.push(rp_app::Cmd::Cover {
                        x: ix,
                        y: iy as i16,
                        pic,
                        color: colour,
                    });
                }
            }
        }
        // The eraser's path: a ring where it is, its trail as dots.
        if self.tool == Tool::Erase && self.live_n > 0 {
            let c = kit::keep(0x80FF_FFFF);
            for &(x, yy) in &self.live[..self.live_n] {
                let sy = i32::from(yy) + self.live_dy + i32::from(iy);
                if (i32::from(TOP)..i32::from(TRAY.y)).contains(&sy) {
                    f.push(rp_app::Cmd::Fill {
                        r: Rect::new(ix + x as u16, sy as u16, 3, 3),
                        color: c,
                    });
                }
            }
            let (x, yy) = self.live[self.live_n - 1];
            let sy = i32::from(yy) + self.live_dy + i32::from(iy);
            if sy >= i32::from(TOP) + 8 {
                f.push(rp_app::Cmd::Outline {
                    r: Rect::new((ix + x as u16).saturating_sub(8), sy as u16 - 8, 17, 17),
                    color: kit::keep(0xC020_2A38),
                });
            }
        }
        if self.menu {
            self.draw_menu(f, cx, &note);
            return;
        }
        self.tray(f, cx, note.id);
    }

    /// The open note's lines from its scroll: paragraphs wrapped on the
    /// rules, checklist items with their round tick, ink blocks as the
    /// design draws them, the caret where typing goes, and the scroll's
    /// place along the right edge.
    fn body(&mut self, f: &mut Frame, cx: &Ctx, id: u32, colour: Argb) {
        let ink: Argb = kit::keep(0xFF20_2A38);
        let view = self.view_rows();
        let bottom = TOP + view * ROW;
        let font = cx.fonts.ui;
        let x0 = PAPER.x + 24;
        let max = PAPER.w - 48;
        // The ink blocks' look under their strokes (the design).
        for v in &self.vis[..self.n_vis] {
            let Item::Ink(b) = v.item else { continue };
            let y = self.screen_y(v.row).max(i32::from(TOP)) as u16;
            let end = (self.screen_y(v.row + v.rows) as u16).min(bottom);
            if end <= y {
                continue;
            }
            match self.design {
                // Framed pads: a sunken sheet of the paper a shade deeper.
                1 => kit::round_rect(
                    f,
                    cx,
                    Rect::new(PAPER.x + 12, y + 2, PAPER.w - 24, end - y - 4),
                    kit::keep(kit::mix(0xFFCE_C8B6, 0xFF70_7890, 0x30)),
                ),
                // Margin marks: the block's bar in the note's colour.
                2 => f.push(rp_app::Cmd::Fill {
                    r: Rect::new(PAPER.x + 12, y + 4, 3, end - y - 8),
                    color: colour,
                }),
                // The open sheet: nothing but the paper (the default).
                _ => {}
            }
            let _ = b;
        }
        let (vis, nv, scroll) = (self.vis, self.n_vis, self.scroll);
        let mut checks = (self.checks, 0usize);
        let caret = if self.typing { self.cursor } else { None };
        let mut caret_at = None;
        if let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok())
            && let Some(store) = guard.as_ref()
        {
            let first = vis[..nv].first().map_or(usize::MAX, |v| v.line);
            let lastl = vis[..nv].last().map_or(0, |v| v.line);
            doc::for_lines(store, id, &mut |k, l| {
                if k < first {
                    return true;
                }
                if k > lastl {
                    return false;
                }
                let Some(v) = vis[..nv].iter().find(|v| v.line == k) else {
                    return true;
                };
                let row_y = |r: u16| i32::from(TOP) + i32::from(ROW) * (i32::from(r) - i32::from(scroll));
                match v.item {
                    Item::Text => {
                        let t = l.strip_prefix(&[doc::CONT]).unwrap_or(l);
                        let mut r = v.row;
                        let mut end_x = x0;
                        wrap(font, t, max, &mut |part| {
                            let y = row_y(r) + i32::from(ROW);
                            if y > i32::from(TOP) && y <= i32::from(bottom) {
                                kit::text(f, x0, y as u16, font, ink, part);
                            }
                            end_x = x0 + font.width(part);
                            r += 1;
                        });
                        if caret == Some(k) {
                            caret_at = Some((end_x, row_y(v.row + v.rows - 1)));
                        }
                    }
                    Item::Check(done) => {
                        let t = &l[4..];
                        let y = row_y(v.row) + i32::from(ROW);
                        if y > i32::from(TOP) && y <= i32::from(bottom) {
                            let y = y as u16;
                            if checks.1 < checks.0.len() {
                                checks.0[checks.1] = (Rect::new(PAPER.x + 16, y - 24, 40, 30), k);
                                checks.1 += 1;
                            }
                            kit::round_check(f, cx, x0, y - 18, self.ring, self.full, done);
                            let col = if done { kit::keep(0x9020_2A38) } else { ink };
                            kit::fitted(f, x0 + 34, y, font, col, t, max - 34);
                            if done {
                                f.push(rp_app::Cmd::Fill {
                                    r: Rect::new(x0 + 34, y - 7, font.width(t).min(max - 34), 1),
                                    color: col,
                                });
                            }
                            if caret == Some(k) {
                                caret_at = Some((x0 + 34 + font.width(t).min(max - 34), row_y(v.row)));
                            }
                        }
                    }
                    Item::Ink(_) => {}
                }
                true
            });
        }
        self.checks = checks.0;
        self.n_checks = checks.1;
        // The caret: typing at the end of its line (or a new one).
        if self.typing {
            let at = caret_at.or_else(|| {
                (self.cursor.is_none()).then(|| (x0, self.screen_y(self.total_rows)))
            });
            if let Some((x, y)) = at
                && y >= i32::from(TOP)
                && y + i32::from(ROW) <= i32::from(bottom)
            {
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(x + 1, y as u16 + 6, 2, 22),
                    color: tint::NOTES,
                });
            }
        }
        // Where the view is in a note longer than the screen.
        if self.total_rows > view {
            let track = view * ROW;
            let h = (u32::from(track) * u32::from(view) / u32::from(self.total_rows)).max(24) as u16;
            let room = track - h;
            let max_scroll = self.total_rows - view;
            let y = TOP + (u32::from(room) * u32::from(self.scroll.min(max_scroll)) / u32::from(max_scroll.max(1))) as u16;
            kit::round_rect(
                f,
                cx,
                Rect::new(PAPER.right() - 9, y, 5, h),
                kit::keep(0x8070_7890),
            );
        }
    }

    /// The menu over the tool strip: its keys, the export formats, or the
    /// files to import (a list over the paper).
    fn draw_menu(&mut self, f: &mut Frame, cx: &Ctx, note: &Row) {
        kit::panel(f, cx, STRIP);
        let words = self.menu_words(note);
        let n = words.iter().filter(|w| !w.is_empty()).count() as u16;
        for (k, w) in words.iter().enumerate().take(usize::from(n)) {
            let r = menu_key(k as u16, n);
            let danger = self.menu_row == 0 && k == 5;
            kit::key(f, cx, r, if danger { kit::KeyState::Down } else { kit::KeyState::Rest });
            kit::centred(f, r, r.y + 23, cx.fonts.label, if danger { kit::LATE } else { TEXT }, w);
        }
        if self.menu_row == 2 {
            let list = Rect::new(PAPER.x + 16, 120, PAPER.w - 32, 40 * 8 + 44);
            kit::panel(f, cx, list);
            kit::text(f, list.x + 14, list.y + 26, cx.fonts.label, DIM, b"IMPORT A TEXT FILE FROM RETROPANE\\NOTES");
            if self.n_files == 0 {
                kit::text(
                    f,
                    list.x + 14,
                    list.y + 70,
                    cx.fonts.ui,
                    DIM,
                    if matches!(self.job, Job::List) { b"Looking..." } else { b"No .txt or .md files there" },
                );
            }
            for (k, name) in self.files[..self.n_files].iter().enumerate() {
                let r = Rect::new(list.x + 8, list.y + 40 + k as u16 * 40, list.w - 16, 36);
                kit::key(f, cx, r, kit::KeyState::Rest);
                kit::fitted(f, r.x + 12, r.y + 24, cx.fonts.ui, TEXT, name.get(), r.w - 24);
            }
        }
    }

    /// The menu row's words.
    fn menu_words(&self, note: &Row) -> [&'static [u8]; 6] {
        match self.menu_row {
            1 => [b"TEXT", b"MARKDOWN", b"SVG", b"PNG", b"BACK", b""],
            2 => [b"BACK", b"", b"", b"", b"", b""],
            _ => [
                if note.pinned { b"UNPIN" } else { b"PIN" },
                b"COLOR",
                [&b"RULED"[..], b"PLAIN", b"GRID"][usize::from(note.paper)],
                b"EXPORT",
                b"IMPORT",
                if self.confirm_delete { b"DELETE?" } else { b"DELETE" },
            ],
        }
    }

    /// The palette tray: the colour wells (the chosen one in the glow), the
    /// three sizes drawn at their size, what a file step or a full note
    /// says, then the tool strip.
    fn tray(&mut self, f: &mut Frame, cx: &Ctx, id: u32) {
        kit::panel(f, cx, TRAY);
        kit::text(f, 24, 472, cx.fonts.label, DIM, b"COLOR");
        for (i, p) in self.wells.iter().enumerate() {
            let r = well(i as u16);
            let chosen = if self.tool == Tool::Marker {
                self.marker_hue.map(usize::from)
            } else {
                Some(usize::from(self.pen))
            };
            if chosen == Some(i) {
                kit::key(f, cx, Rect::new(r.x - 5, r.y - 5, 50, 50), kit::KeyState::On);
            }
            if let Some(p) = *p {
                f.push(rp_app::Cmd::Pic {
                    x: r.x,
                    y: r.y as i16,
                    pic: p,
                });
            }
            // The highlighter's wells: its see-through band over each.
            if self.tool == Tool::Marker {
                let hue = stroke_colour(MARKER | MARKER_HUE | i as u8);
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(r.x - 2, r.y + 14, r.w + 4, 12),
                    color: hue,
                });
            }
        }
        let note: &[u8] = if self.text_full {
            b"THIS NOTE'S TEXT IS FULL"
        } else if self.ink_full {
            b"THIS NOTE'S INK IS FULL"
        } else {
            self.said
        };
        if note.is_empty() {
            kit::text(f, 326, 472, cx.fonts.label, DIM, b"SIZE");
        } else {
            kit::right(f, 456, 472, cx.fonts.label, kit::LATE, note);
        }
        for (i, d) in [6u16, 12, 20].iter().enumerate() {
            let r = size_key(i as u16);
            kit::key(
                f,
                cx,
                r,
                if i == usize::from(self.size) { kit::KeyState::On } else { kit::KeyState::Rest },
            );
            kit::round_rect(
                f,
                cx,
                Rect::new(r.x + (44 - d) / 2, r.y + (44 - d) / 2, *d, *d),
                if cx.look.light { kit::keep(0xFF28_3038) } else { TEXT },
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
            let r = tool_key(k);
            kit::key(f, cx, r, if k == chosen { kit::KeyState::On } else { kit::KeyState::Rest });
            // REDO is dim while there is nothing to put back.
            let col = if k == 6 && (self.redo_n == 0 || self.redo_of != Some(id)) {
                kit::alpha(DIM, 0x80)
            } else {
                TEXT
            };
            kit::centred(f, r, r.y + 23, cx.fonts.label, col, t);
        }
    }
}

/// Menu key `k` of `n` across the strip.
const fn menu_key(k: u16, n: u16) -> Rect {
    let w = (STRIP.w - 12) / n;
    Rect::new(STRIP.x + 6 + k * w, STRIP.y + 6, w - 4, 36)
}

/// `t` wrapped to `max` pixels in `font` (at spaces, or anywhere in a word
/// longer than the line), each row given to `f`: how many rows (one at
/// least, for an empty line).
fn wrap(font: &rp_app::Font, t: &[u8], max: u16, f: &mut dyn FnMut(&[u8])) -> u16 {
    if t.is_empty() {
        f(t);
        return 1;
    }
    let (mut start, mut rows) = (0usize, 0u16);
    while start < t.len() {
        let (mut end, mut w, mut space) = (start, 0u16, None);
        while end < t.len() {
            let cw = font.width(&t[end..end + 1]);
            if w + cw > max && end > start {
                break;
            }
            w += cw;
            if t[end] == b' ' {
                space = Some(end);
            }
            end += 1;
        }
        let cut = if end < t.len() {
            space.filter(|&s| s > start).map_or(end, |s| s + 1)
        } else {
            end
        };
        // A wrapped row starts on a character, never inside one.
        let mut cut = cut;
        while cut < t.len() && cut > start + 1 && t[cut] & 0xC0 == 0x80 {
            cut -= 1;
        }
        f(&t[start..cut]);
        rows = rows.saturating_add(1);
        start = cut;
    }
    rows
}

impl Notes {
    /// The stroke being written, its newest segment painted into the
    /// note's ink layer at its block's place with the saved ink's brush
    /// (the owner, 2026-11-14: "the strokes only showed up as dots, not as
    /// fluid strokes"). The last three points: the spline's segment before
    /// the newest is painted again with its neighbour known, the brush
    /// keeps the most coverage, so nothing doubles.
    fn live_paint(&mut self, cx: &Ctx) {
        if self.tool == Tool::Erase {
            return;
        }
        let Some(pics) = cx.pics else { return };
        let style = self.style();
        let colour = stroke_colour(style);
        let Some(l) = self.layer_for(pics, colour) else {
            return;
        };
        // Renewed before each change: a picture once drawn is final.
        self.ink[l] = self.ink[l].and_then(|(c, p)| pics.renew(p).map(|p| (c, p)));
        let Some((_, p)) = self.ink[l] else { return };
        let n = self.live_n;
        let from = n.saturating_sub(3);
        let pts = &self.live[from..n];
        let marker = style & MARKER != 0;
        let ws: &[u8] = if marker { &[] } else { &self.live_w[from..n] };
        let dy = self.live_dy;
        pics.write(p, |w| paint_cover_at(w, &[pts], &[style], &[ws], dy, colour));
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
            MARKER | self.marker_hue.map_or(0, |h| MARKER_HUE | h)
        } else {
            self.pen
        }
    }

    /// The stroke just written added to the note's ink (or, with the
    /// eraser, the stroke of its block it crossed taken out).
    fn end_stroke(&mut self, cx: &Ctx, id: u32) {
        let (mut live, mut ws, tool, style) = (self.live, self.live_w, self.tool, self.style());
        // Thinned as it is kept (the eraser's path as it was: it is only
        // compared).
        let n = if tool == Tool::Erase {
            self.live_n
        } else {
            simplify(&mut live, &mut ws, self.live_n)
        };
        self.live_n = 0;
        if n == 0 {
            return;
        }
        let block = self.live_block;
        let (page, np) = pages_of(cx, id);
        let kept = if tool == Tool::Erase {
            // The note's own strokes first, then each page's, as they are
            // painted.
            ink_op(cx, id, InkOp::Erase(&live[..n], block), false)
                || page[..np]
                    .iter()
                    .any(|&p| ink_op(cx, p, InkOp::Erase(&live[..n], block), true))
        } else {
            // Packed once (a stroke is 7 + 2 x 254 + 128 bytes at most),
            // then added at the ink's end: the last page's, or the note's
            // own while it has none; where that is full, a new page.
            let mut packed = [0u8; STROKE_MAX];
            let widths = (style & MARKER == 0).then_some(&ws[..n]);
            let Some(len) = encode_stroke(&live[..n], widths, style, block, &mut packed) else {
                return;
            };
            // Something new written: nothing to put back any more.
            self.redo_n = 0;
            add_stroke(cx, id, &packed[..len])
        };
        // A stroke that didn't fit is said, not dropped in silence (the
        // owner, 2026-11-14: strokes stopped showing with no word why).
        if tool != Tool::Erase {
            self.ink_full = !kept;
        } else if kept {
            self.ink_full = false;
        }
    }

    /// `bytes` of `kind` (0 a stroke, 1 a character) kept for REDO.
    fn remember(&mut self, id: u32, bytes: &[u8], kind: u8) {
        if self.redo_of != Some(id) {
            self.redo_of = Some(id);
            self.redo_n = 0;
        }
        // No room: the oldest are forgotten (all of them: an undo that
        // deep is rare, and half a history would put strokes back out of
        // order).
        if self.redo_n + bytes.len() + 3 > self.redo.len() {
            self.redo_n = 0;
        }
        let n = self.redo_n;
        let Some(to) = self.redo.get_mut(n..n + bytes.len() + 3) else {
            return;
        };
        to[..bytes.len()].copy_from_slice(bytes);
        to[bytes.len()..bytes.len() + 2].copy_from_slice(&(bytes.len() as u16).to_le_bytes());
        to[bytes.len() + 2] = kind;
        self.redo_n += bytes.len() + 3;
    }

    /// The last stroke taken out (UNDO), or the last character typed;
    /// kept for REDO.
    fn undo(&mut self, cx: &Ctx, id: u32) {
        // The last stroke is the last page's, or the note's own; with
        // no ink, the last character typed.
        let (page, np) = pages_of(cx, id);
        let (rec, is_page) = match page[..np].last() {
            Some(&p) => (p, true),
            None => (id, false),
        };
        let mut last = [0u8; STROKE_MAX];
        let len = last_stroke(cx, rec, &mut last);
        if len > 0 && ink_op(cx, rec, InkOp::Last, is_page) {
            self.remember(id, &last[..len], 0);
            return;
        }
        self.untype(cx, id);
    }

    /// The line typing goes into: the caret's text or checklist line, else
    /// a new line after it (or at the note's end).
    fn typing_line(&mut self, cx: &Ctx, id: u32) -> Option<usize> {
        if let Some(k) = self.cursor
            && self.line_item(cx, id, k).is_some_and(|i| !matches!(i, Item::Ink(_)))
        {
            return Some(k);
        }
        let at = self.cursor.map_or(self.n_lines, |k| k + 1).min(self.n_lines);
        text_op(cx, id, &doc::Edit::Insert(at, b"")).ok()?;
        // An empty note holds no empty line: the first character makes it.
        self.n_lines = cx
            .pim
            .and_then(|c| c.try_borrow().ok())
            .and_then(|g| g.as_ref().map(|s| doc::line_count(s, id)))
            .unwrap_or(0);
        self.cursor = Some(at);
        Some(at)
    }

    /// What line `k` of note `id` is.
    fn line_item(&self, cx: &Ctx, id: u32, k: usize) -> Option<Item> {
        let g = cx.pim?.try_borrow().ok()?;
        let store = g.as_ref()?;
        let mut b = [0u8; 4];
        let n = doc::line(store, id, k, &mut b)?;
        let l = &b[..n];
        Some(match doc::ink_block(l) {
            Some(x) => Item::Ink(x),
            None if l.starts_with(b"[ ] ") => Item::Check(false),
            None if l.starts_with(b"[x] ") => Item::Check(true),
            None => Item::Text,
        })
    }

    /// The last character of the caret's line taken off (the keyboard's
    /// backspace, and UNDO with no ink left); kept for REDO. An emptied
    /// line (or an empty checklist item) goes, the caret to the line
    /// before.
    fn untype(&mut self, cx: &Ctx, id: u32) {
        let k = match self.cursor {
            Some(k) => k,
            None => {
                // The last line, unless it is ink.
                let k = self.n_lines.checked_sub(1);
                match k {
                    Some(k) if !matches!(self.line_item(cx, id, k), Some(Item::Ink(_))) => k,
                    _ => return,
                }
            }
        };
        if matches!(self.line_item(cx, id, k), Some(Item::Ink(_)) | None) {
            return;
        }
        let mut b = [0u8; 8];
        let n = cx
            .pim
            .and_then(|c| c.try_borrow().ok())
            .and_then(|g| g.as_ref().and_then(|s| doc::line(s, id, k, &mut b)))
            .unwrap_or(0);
        let empty = n == 0 || (n == 4 && (b[..4] == *b"[ ] " || b[..4] == *b"[x] "));
        if empty {
            if text_op(cx, id, &doc::Edit::Remove(k)).is_ok() {
                self.remember(id, b"\n", 1);
                self.cursor = k.checked_sub(1);
                self.n_lines = self.n_lines.saturating_sub(1);
            }
            return;
        }
        if let Ok((c, m)) = text_op(cx, id, &doc::Edit::Pop(k)) {
            self.remember(id, &c[..m], 1);
            self.cursor = Some(k);
        }
    }

    /// What UNDO last took out put back (REDO): the stroke where strokes
    /// are added (its block in it), the character at the caret.
    fn redo(&mut self, cx: &Ctx, id: u32) {
        if self.redo_of != Some(id) || self.redo_n < 3 {
            return;
        }
        let n = self.redo_n;
        let kind = self.redo[n - 1];
        let len = usize::from(u16::from_le_bytes([self.redo[n - 3], self.redo[n - 2]]));
        let Some(from) = n.checked_sub(3 + len) else {
            self.redo_n = 0;
            return;
        };
        let mut bytes = [0u8; STROKE_MAX];
        let Some(to) = bytes.get_mut(..len) else {
            self.redo_n = 0;
            return;
        };
        to.copy_from_slice(&self.redo[from..from + len]);
        let back = if kind == 0 {
            add_stroke(cx, id, &bytes[..len])
        } else if bytes[..len] == *b"\n" {
            let at = self.cursor.map_or(self.n_lines, |k| k + 1);
            let ok = text_op(cx, id, &doc::Edit::Insert(at, b"")).is_ok();
            if ok {
                self.cursor = Some(at);
            }
            ok
        } else {
            self.typing_line(cx, id)
                .is_some_and(|k| append(cx, id, k, &bytes[..len]))
        };
        if back {
            self.redo_n = from;
        }
    }

    /// Text typed into the note at the caret.
    fn type_char(&mut self, cx: &Ctx, id: u32, c: u8) {
        let Some(k) = self.typing_line(cx, id) else {
            self.text_full = true;
            return;
        };
        self.text_full = !append(cx, id, k, &[c]);
        self.follow = true;
    }

    /// Return: a new line after the caret's (a new item in a checklist; an
    /// empty item ends the list).
    fn new_line(&mut self, cx: &Ctx, id: u32) {
        let k = self.typing_line(cx, id);
        let item = k.and_then(|k| self.line_item(cx, id, k));
        let at = k.map_or(self.n_lines, |k| k + 1);
        let mut b = [0u8; 8];
        let n = k
            .and_then(|k| {
                cx.pim
                    .and_then(|c| c.try_borrow().ok())
                    .and_then(|g| g.as_ref().and_then(|s| doc::line(s, id, k, &mut b)))
            })
            .unwrap_or(0);
        if matches!(item, Some(Item::Check(_))) && n == 4 {
            // An empty item: the list ends, the line a paragraph.
            if let Some(k) = k {
                let _ = text_op(cx, id, &doc::Edit::Set(k, b""));
                self.tool = Tool::Text;
            }
            return;
        }
        let start: &[u8] = if matches!(item, Some(Item::Check(_))) { b"[ ] " } else { b"" };
        if text_op(cx, id, &doc::Edit::Insert(at, start)).is_ok() {
            self.cursor = Some(at);
            self.n_lines += 1;
            self.follow = true;
        } else {
            self.text_full = true;
        }
    }

    /// Checklist line `line` ticked or unticked.
    fn toggle(&mut self, cx: &Ctx, id: u32, line: usize) {
        let mut b = [0u8; 2100];
        let Some(n) = cx
            .pim
            .and_then(|c| c.try_borrow().ok())
            .and_then(|g| g.as_ref().and_then(|s| doc::line(s, id, line, &mut b)))
        else {
            return;
        };
        if n < 4 || !(b.starts_with(b"[ ] ") || b.starts_with(b"[x] ")) {
            return;
        }
        b[1] = if b[1] == b'x' { b' ' } else { b'x' };
        let _ = text_op(cx, id, &doc::Edit::Set(line, &b[..n]));
    }

    /// A new note, empty, its ink block ready: its place in the list.
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
        self.opened();
        store
            .all(Kind::Note)
            .position(|r| r.id == id)
            .map(|k| k as u8)
    }

    /// A note opened: at its top, the caret at its end, the pen ready.
    fn opened(&mut self) {
        self.scroll = 0;
        self.cursor = None;
        self.typing = false;
        self.menu = false;
        self.menu_row = 0;
        self.said = b"";
        self.tool = Tool::Pen;
        self.painted = None;
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

    /// The stylus down on the open note's paper: a stroke starts in the
    /// ink block under it (or, on the blank paper below the last line, in
    /// the last block when it is ink and close, else a new block), or a
    /// tap or a drag begins.
    fn pen_down(&mut self, cx: &Ctx, id: u32, x: u16, y: u16) -> Reply {
        let rail = x >= PAPER.right() - RAIL;
        let ink_tool = matches!(self.tool, Tool::Pen | Tool::Marker | Tool::Erase);
        let target = if rail || !ink_tool {
            None
        } else {
            match self.item_at(y) {
                Some(Vis { item: Item::Ink(b), row, .. }) => Some((b, row)),
                Some(_) => None,
                None if self.tool == Tool::Erase => None,
                None => match self.last {
                    // Below an ink block that ends the note: it grows
                    // there (a new note's sheet takes ink all the way to
                    // the tray, as it always has).
                    Some((Item::Ink(b), row)) => Some((b, row)),
                    // Else a new block where the note ends.
                    _ => {
                        let b = self.next_block;
                        let (line, n) = doc::ink_line(b);
                        text_op(cx, id, &doc::Edit::Insert(self.n_lines, &line[..n]))
                            .ok()
                            .map(|_| {
                                self.n_lines += 1;
                                (b, self.total_rows)
                            })
                    }
                },
            }
        };
        match target {
            Some((b, row)) => {
                self.live_block = b;
                self.live_dy = i32::from(ROW) * (i32::from(row) - i32::from(self.scroll));
                self.inking = true;
                self.typing = false;
                self.tap = None;
                let local = self.local(x, y);
                let (p, w) = self.filt.start(local.0, local.1);
                self.live[0] = p;
                self.live_w[0] = w;
                self.live_n = 1;
                self.live_paint(cx);
            }
            None => {
                self.inking = false;
                self.tap = Some((x, y, false));
            }
        }
        Reply::Redraw
    }

    /// A screen point in the stroke's block's own coordinates (inside it
    /// across, never above its top).
    fn local(&self, x: u16, y: u16) -> (i16, i16) {
        let lx = (i32::from(x) - i32::from(INK_AT.0)).clamp(0, i32::from(INK_W) - 1);
        let ly = (i32::from(y) - i32::from(INK_AT.1) - self.live_dy).clamp(0, 32_000);
        (lx as i16, ly as i16)
    }

    /// The stylus moved on the paper: the stroke goes on (filtered), or a
    /// drag scrolls a row at a time.
    fn pen_move(&mut self, cx: &Ctx, x: u16, y: u16) -> Reply {
        if self.inking {
            if self.live_n < LIVE {
                let local = self.local(x, y);
                let (p, w) = self.filt.step(local.0, local.1);
                self.live[self.live_n] = p;
                self.live_w[self.live_n] = w;
                self.live_n += 1;
                self.live_paint(cx);
            }
            return Reply::Redraw;
        }
        let Some((x0, y0, _)) = self.tap else {
            return Reply::Nothing;
        };
        let d = i32::from(y) - i32::from(y0);
        if d.abs() < i32::from(ROW) {
            return Reply::Nothing;
        }
        let steps = (d / i32::from(ROW)) as i16;
        let max = self.total_rows.saturating_sub(self.view_rows()) as i32;
        let s = (i32::from(self.scroll) - i32::from(steps)).clamp(0, max.max(0)) as u16;
        self.tap = Some((x0, (i32::from(y0) + i32::from(steps) * i32::from(ROW)) as u16, true));
        if s == self.scroll {
            return Reply::Nothing;
        }
        self.scroll = s;
        Reply::Redraw
    }

    /// The stylus lifted off the paper: the stroke kept, or a tap placed
    /// (a tick box ticks; a line takes the caret).
    fn pen_up(&mut self, cx: &Ctx, id: u32, x: u16, y: u16) -> Reply {
        if self.inking {
            self.inking = false;
            if self.live_n > 0 && self.live_n < LIVE {
                let local = self.local(x, y);
                let last = self.live[self.live_n - 1];
                if (local.0 - last.0).abs() + (local.1 - last.1).abs() > 0 {
                    let (p, w) = self.filt.lift(local.0, local.1);
                    self.live[self.live_n] = p;
                    self.live_w[self.live_n] = w;
                    self.live_n += 1;
                }
            }
            self.end_stroke(cx, id);
            return Reply::Redraw;
        }
        let Some((_, _, dragged)) = self.tap.take() else {
            return Reply::Nothing;
        };
        if dragged || x >= PAPER.right() - RAIL {
            return Reply::Nothing;
        }
        if let Some(&(_, line)) = self.checks[..self.n_checks]
            .iter()
            .find(|(r, _)| r.contains_point(x, y))
        {
            self.toggle(cx, id, line);
            return Reply::Redraw;
        }
        match self.item_at(y) {
            Some(v) if !matches!(v.item, Item::Ink(_)) => {
                self.cursor = Some(v.line);
                self.typing = true;
                self.tool = if matches!(v.item, Item::Check(_)) { Tool::List } else { Tool::Text };
            }
            None => {
                // Below the last line: typing goes on at the end.
                self.cursor = self.n_lines.checked_sub(1).filter(|&k| {
                    !matches!(self.line_item(cx, id, k), Some(Item::Ink(_)))
                });
                if self.cursor.is_none() {
                    self.cursor = None;
                }
                self.typing = true;
                self.tool = Tool::Text;
            }
            Some(_) => return Reply::Nothing,
        }
        Reply::Redraw
    }

    /// A file step begun: note `id` exported as `fmt` (each ink block with
    /// strokes, one file each, for SVG and PNG).
    fn export(&mut self, cx: &Ctx, id: u32, fmt: export::Format) {
        let left = match fmt {
            export::Format::Svg | export::Format::Png => {
                let m = cx
                    .pim
                    .and_then(|c| c.try_borrow().ok())
                    .and_then(|g| g.as_ref().map(|s| export::inked_blocks(s, id)))
                    .unwrap_or(0);
                if m == 0 {
                    self.said = b"NO INK TO EXPORT";
                    return;
                }
                m
            }
            _ => 1,
        };
        self.next_file(cx, id, fmt, left);
    }

    /// The next file of an export: its length counted, the shell asked to
    /// write it.
    fn next_file(&mut self, cx: &Ctx, id: u32, fmt: export::Format, left: u128) {
        let ink = matches!(fmt, export::Format::Svg | export::Format::Png);
        let block = if ink { left.trailing_zeros() as u8 } else { 0 };
        let mut count = export::Count::default();
        let mut title = Text::<64>::EMPTY;
        let mut png = core::mem::replace(&mut self.png, export::PngWork::new());
        if let Some(g) = cx.pim.and_then(|c| c.try_borrow().ok())
            && let Some(store) = g.as_ref()
        {
            title = Text::of(store.get(id).map_or(&[][..], |r| r.text(tag::TITLE)));
            match fmt {
                export::Format::Svg => export::svg(store, id, block, &mut count),
                export::Format::Png => export::png(store, id, block, &mut png, &mut count),
                _ => export::text(store, id, title.get(), fmt, &mut count),
            }
        }
        self.png = png;
        let mut name = [0u8; 64];
        let n = export::file_name(title.get(), if ink { block + 1 } else { 0 }, fmt, &mut name);
        self.job = Job::Write {
            id,
            fmt,
            block,
            left: left & !(1u128 << block),
        };
        self.asking = Some(rp_app::FileAsk::Write {
            dir: b"Notes",
            name: Text::of(&name[..n]),
            len: count.0 as u32,
        });
        self.said = b"SAVING...";
    }

    /// An import's piece: its lines put at the note's end as they come; a
    /// damaged file taken back out whole and named.
    fn import_piece(&mut self, cx: &Ctx, off: u32, total: u32, bytes: &[u8]) -> Reply {
        let Job::Read { id, put } = self.job else {
            return Reply::Nothing;
        };
        let mut put = put;
        let fail = |me: &mut Self, put: u32, d: import::Damage| {
            for _ in 0..put {
                let _ = text_op(cx, id, &doc::Edit::DropLast);
            }
            me.job = Job::None;
            me.said = d.says();
            me.menu = false;
            me.menu_row = 0;
        };
        if off == 0 {
            if total > import::MAX_FILE {
                fail(self, 0, import::Damage::TooBig(total));
                return Reply::Redraw;
            }
            self.reader = import::Reader::new();
        }
        let mut sink = |l: &[u8]| -> Result<(), import::Damage> {
            text_op(cx, id, &doc::Edit::Push(l)).map_err(|_| import::Damage::NoRoom)?;
            put += 1;
            Ok(())
        };
        let mut r = self.reader.feed(bytes, &mut sink);
        let end = off as usize + bytes.len() >= total as usize;
        if r.is_ok() && end {
            r = self.reader.finish(&mut sink);
        }
        match r {
            Err(d) => fail(self, put, d),
            Ok(()) if end => {
                self.job = Job::None;
                self.said = b"IMPORTED";
                self.menu = false;
                self.menu_row = 0;
                self.follow = true;
                self.cursor = None;
            }
            Ok(()) => self.job = Job::Read { id, put },
        }
        Reply::Redraw
    }

    /// The suggestions for the word before the caret, and whether the next
    /// letter starts a sentence, from the caret's line.
    fn suggest_now(&mut self, cx: &Ctx, id: u32) {
        self.sugg_n = [0; 3];
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return;
        };
        let Some(store) = guard.as_ref() else { return };
        if self.learned_at != Some(store.changes()) {
            // The user's own words, counted from every note.
            self.learned.clear();
            let dict = words::Dict::default_lang();
            for r in store.all(Kind::Note) {
                doc::for_lines(store, r.id, &mut |_, l| {
                    self.learned.count_in(l, dict.as_ref());
                    true
                });
            }
            self.learned_at = Some(store.changes());
        }
        let mut b = [0u8; 2100];
        let n = self
            .cursor
            .and_then(|k| doc::line(store, id, k, &mut b))
            .unwrap_or(0);
        let line = &b[..n];
        self.caps_next = words::sentence_starts(line);
        let start = line
            .iter()
            .rposition(|c| !(c.is_ascii_alphabetic() || *c == b'\''))
            .map_or(0, |i| i + 1);
        let word = &line[start..];
        let dict = words::Dict::default_lang();
        self.sugg_n = words::suggest(word, dict.as_ref(), &self.learned, &mut self.sugg);
    }

    /// Suggestion `k` taken: the word being typed finished with it and a
    /// space.
    fn take_suggestion(&mut self, cx: &Ctx, id: u32, k: usize) {
        let Some(&n) = self.sugg_n.get(k) else { return };
        let Some(line) = self.cursor else { return };
        if n == 0 {
            return;
        }
        let mut b = [0u8; 2100];
        let Some(len) = cx
            .pim
            .and_then(|c| c.try_borrow().ok())
            .and_then(|g| g.as_ref().and_then(|s| doc::line(s, id, line, &mut b)))
        else {
            return;
        };
        let start = b[..len]
            .iter()
            .rposition(|c| !(c.is_ascii_alphabetic() || *c == b'\''))
            .map_or(0, |i| i + 1);
        let typed = len - start;
        let word = self.sugg[k];
        if n > typed {
            let mut rest = [0u8; words::WORD_MAX + 1];
            rest[..n - typed].copy_from_slice(&word[typed..n]);
            rest[n - typed] = b' ';
            // The word's own capitals: what was typed is replaced as is.
            let _ = append(cx, id, line, &rest[..n - typed + 1]);
        }
    }
}

/// `bytes` put at the end of line `k` of note `id` (an empty note's first
/// line made).
fn append(cx: &Ctx, id: u32, k: usize, bytes: &[u8]) -> bool {
    match text_op(cx, id, &doc::Edit::Append(k, bytes)) {
        Ok(_) => true,
        Err(doc::Refused::NoLine) => text_op(cx, id, &doc::Edit::Insert(k, bytes)).is_ok(),
        Err(_) => false,
    }
}

/// A packed stroke's most bytes: its head and extension, 254 steps and a
/// width nibble a point.
const STROKE_MAX: usize = 7 + 2 * 254 + 128;

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
        if let Some(id) = self.open_id() {
            self.layout(cx, id);
        }
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
        let reply = self.event_in(e, cx, id);
        // What typing changed: the suggestions and the next capital.
        if let (Some(id), true) = (self.open_id(), self.typing)
            && matches!(e, Event::Char(_) | Event::Delete | Event::Done | Event::Suggestion(_) | Event::PenUp { .. })
        {
            self.suggest_now(cx, id);
        }
        reply
    }

    fn keyboard(&self) -> Option<Field> {
        match self.view {
            View::Home if self.searching => Some(Field::Line),
            View::Note(_) if self.typing => Some(Field::Sentence),
            _ => None,
        }
    }

    fn suggestions(&self) -> [&[u8]; 3] {
        if !self.typing {
            return [&[], &[], &[]];
        }
        [
            &self.sugg[0][..self.sugg_n[0]],
            &self.sugg[1][..self.sugg_n[1]],
            &self.sugg[2][..self.sugg_n[2]],
        ]
    }

    fn caps(&self) -> Option<bool> {
        (self.typing && matches!(self.view, View::Note(_))).then_some(self.caps_next)
    }

    fn file_ask(&mut self) -> Option<rp_app::FileAsk> {
        self.asking.take()
    }

    fn file_piece(&mut self, off: u32, out: &mut [u8], cx: &Ctx) -> usize {
        let Job::Write { id, fmt, block, .. } = self.job else {
            return 0;
        };
        let Some(g) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return 0;
        };
        let Some(store) = g.as_ref() else { return 0 };
        let mut w = export::Window::new(off as usize, out);
        let mut title = [0u8; 64];
        let tn = store.get(id).map_or(0, |r| {
            let t = r.text(tag::TITLE);
            let n = t.len().min(64);
            title[..n].copy_from_slice(&t[..n]);
            n
        });
        match fmt {
            export::Format::Svg => export::svg(store, id, block, &mut w),
            export::Format::Png => export::png(store, id, block, &mut self.png, &mut w),
            _ => export::text(store, id, &title[..tn], fmt, &mut w),
        }
        w.got
    }

    fn file_answer(&mut self, a: rp_app::FileAnswer<'_>, cx: &Ctx) -> Reply {
        match (a, self.job) {
            (rp_app::FileAnswer::Failed(why), _) => {
                self.job = Job::None;
                self.said = why.as_bytes();
                Reply::Redraw
            }
            (rp_app::FileAnswer::Wrote, Job::Write { id, fmt, left, .. }) => {
                if left != 0 {
                    self.next_file(cx, id, fmt, left);
                } else {
                    self.job = Job::None;
                    self.said = b"SAVED IN RETROPANE\\NOTES";
                }
                Reply::Redraw
            }
            (rp_app::FileAnswer::Listed(names), Job::List) => {
                self.n_files = 0;
                for n in names.split(|&c| c == b'\n') {
                    let low = |e: &[u8]| n.len() > e.len() && n[n.len() - e.len()..].eq_ignore_ascii_case(e);
                    if (low(b".txt") || low(b".md")) && self.n_files < self.files.len() {
                        self.files[self.n_files] = Text::of(n);
                        self.n_files += 1;
                    }
                }
                self.job = Job::None;
                Reply::Redraw
            }
            (rp_app::FileAnswer::Piece { off, total, bytes }, Job::Read { .. }) => {
                self.import_piece(cx, off, total, bytes)
            }
            _ => Reply::Nothing,
        }
    }
}

impl Notes {
    /// The events, past the D-pad's walk.
    #[allow(clippy::too_many_lines)]
    fn event_in(&mut self, e: Event, cx: &Ctx, id: Option<u32>) -> Reply {
        match (e, self.view) {
            (Event::Key(Key::SoftLeft), View::Home) => Reply::Close,
            (Event::Key(Key::SoftLeft), _) => {
                if self.menu {
                    if self.menu_row != 0 {
                        self.menu_row = 0;
                    } else {
                        self.menu = false;
                    }
                } else if self.typing {
                    self.typing = false;
                } else {
                    self.view = View::Home;
                }
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::Note(_)) => {
                self.menu = !self.menu;
                self.menu_row = 0;
                self.confirm_delete = false;
                self.focus.clear();
                Reply::Redraw
            }
            // A file to import, from the list over the paper.
            (Event::PenUp { x, y }, View::Note(_))
                if self.menu && self.menu_row == 2 && y >= 160 && y < 160 + 40 * 8 && !STRIP.contains_point(x, y) =>
            {
                let k = usize::from((y - 160) / 40);
                if let (Some(id), true) = (id, k < self.n_files) {
                    self.job = Job::Read { id, put: 0 };
                    self.asking = Some(rp_app::FileAsk::Read {
                        dir: b"Notes",
                        name: self.files[k],
                    });
                    self.said = b"IMPORTING...";
                    self.menu = false;
                    self.menu_row = 0;
                }
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Note(i)) if self.menu && STRIP.contains_point(x, y) => {
                let Some(row) = self.rows.get(usize::from(i)).copied().flatten() else {
                    return Reply::Nothing;
                };
                let words = self.menu_words(&row);
                let n = words.iter().filter(|w| !w.is_empty()).count() as u16;
                let Some(k) = (0..n).find(|&k| menu_key(k, n).contains_point(x, y)) else {
                    return Reply::Nothing;
                };
                match (self.menu_row, k) {
                    (1, 0..=3) => {
                        let fmt = [
                            export::Format::Text,
                            export::Format::Markdown,
                            export::Format::Svg,
                            export::Format::Png,
                        ][usize::from(k)];
                        self.export(cx, row.id, fmt);
                        self.menu = false;
                        self.menu_row = 0;
                    }
                    (1 | 2, _) => self.menu_row = 0,
                    (_, 0) => self.set_num(cx, row.id, tag::PINNED, (!row.pinned).then_some(1)),
                    (_, 1) => {
                        let c = kit::IDENTITY
                            .iter()
                            .position(|&x| x == row.colour)
                            .unwrap_or(1);
                        self.set_num(cx, row.id, tag::COLOUR, Some(((c + 1) % 8) as u32));
                    }
                    (_, 2) => self.set_num(cx, row.id, tag::PAPER, Some(u32::from((row.paper + 1) % 3))),
                    (_, 3) => self.menu_row = 1,
                    (_, 4) => {
                        self.menu_row = 2;
                        self.n_files = 0;
                        self.job = Job::List;
                        self.asking = Some(rp_app::FileAsk::List { dir: b"Notes" });
                    }
                    _ if self.confirm_delete => {
                        self.delete_note(cx, row.id);
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
            // The stylus down on a sheet: held half a second, Delete is
            // offered for that note (the owner, 2026-11-15: "you should
            // be able to long press ... on any of the notes, and delete
            // them"; Windows Mobile's tap-and-hold menu, ours one key on
            // the sheet itself).
            (Event::PenDown { x, y }, View::Home) => {
                self.down_y = Some(y);
                self.hold = self.cards[..self.n_cards]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                    .map(|c| (c.1, 0));
                if self.hold.is_some() && self.ask.is_none() {
                    Reply::Animate
                } else {
                    Reply::Nothing
                }
            }
            (Event::PenMove { y, .. }, View::Home) => {
                // A drag is no hold.
                if self.down_y.is_some_and(|y0| y0.abs_diff(y) > 8) {
                    self.hold = None;
                }
                Reply::Nothing
            }
            (Event::Tick, View::Home) => match self.hold {
                Some((place, t)) if t + 1 >= HOLD_TICKS => {
                    self.hold = None;
                    self.ask = Some(place);
                    self.ask_held = true;
                    Reply::Redraw
                }
                Some((place, t)) => {
                    self.hold = Some((place, t + 1));
                    Reply::Animate
                }
                None => Reply::Nothing,
            },
            // The lift that ends the hold leaves the question up; the next
            // tap answers it: on its key the note goes, anywhere else it
            // stays.
            (Event::PenUp { .. }, View::Home) if self.ask_held => {
                self.ask_held = false;
                self.down_y = None;
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Home) if self.ask.is_some() => {
                self.down_y = None;
                self.hold = None;
                let place = self.ask.take();
                if self.ask_key.contains_point(x, y)
                    && let Some(row) =
                        place.and_then(|p| self.rows.get(usize::from(p)).copied().flatten())
                {
                    self.delete_note(cx, row.id);
                }
                Reply::Redraw
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
                    } else if !self.page_on() {
                        return Reply::Nothing;
                    }
                } else if self.sel > self.first {
                    self.sel -= 1;
                } else if !self.page_back() {
                    // Nothing higher: the shell takes the arrow (Start).
                    self.keys = false;
                    return Reply::Nothing;
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
                self.opened();
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
                    self.opened();
                }
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Home) => {
                self.hold = None;
                // A card opens its note.
                match self.cards[..self.n_cards]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                {
                    Some(&(_, k)) => {
                        self.view = View::Note(k);
                        self.opened();
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
                    // The marker takes the colour and stays the marker;
                    // any other tool gives way to the pen in it.
                    if self.tool == Tool::Marker {
                        self.marker_hue = Some(i as u8);
                    } else {
                        self.pen = i as u8;
                        if self.tool != Tool::Pen {
                            self.tool = Tool::Pen;
                            self.typing = false;
                        }
                    }
                } else if let Some(i) = (0..3u16).find(|&i| size_key(i).contains_point(x, y)) {
                    self.size = i as u8;
                }
                Reply::Redraw
            }
            // The tool strip.
            (Event::PenUp { x, y }, View::Note(_)) if STRIP.contains_point(x, y) => {
                let Some(k) = (0..TOOLS.len()).find(|&k| tool_key(k).contains_point(x, y)) else {
                    return Reply::Nothing;
                };
                match k {
                    0 => self.tool = Tool::Pen,
                    1 => self.tool = Tool::Marker,
                    2 => self.tool = Tool::Erase,
                    3 | 4 => {
                        // A new paragraph, or a new checklist item, after
                        // the caret's line (at the end with none).
                        self.tool = if k == 3 { Tool::Text } else { Tool::List };
                        self.typing = true;
                        if let Some(id) = id {
                            let at = self.cursor.map_or(self.n_lines, |c| c + 1).min(self.n_lines);
                            let start: &[u8] = if k == 3 { b"" } else { b"[ ] " };
                            if text_op(cx, id, &doc::Edit::Insert(at, start)).is_ok() {
                                self.cursor = Some(at);
                                self.n_lines += 1;
                                self.follow = true;
                            } else if k == 4 || self.n_lines > 0 {
                                self.text_full = true;
                            }
                        }
                    }
                    5 => {
                        if let Some(id) = id {
                            self.undo(cx, id);
                        }
                    }
                    _ => {
                        if let Some(id) = id {
                            self.redo(cx, id);
                        }
                    }
                }
                Reply::Redraw
            }
            // A tick box tapped (the D-pad's centre lifts only).
            (Event::PenUp { x, y }, View::Note(_))
                if !self.inking
                    && !self.menu
                    && self.checks[..self.n_checks].iter().any(|(r, _)| r.contains_point(x, y)) =>
            {
                self.tap = None;
                if let (Some(id), Some(&(_, line))) = (
                    id,
                    self.checks[..self.n_checks].iter().find(|(r, _)| r.contains_point(x, y)),
                ) {
                    self.toggle(cx, id, line);
                }
                Reply::Redraw
            }
            // The paper: writing, ticking, placing the caret, scrolling.
            (Event::PenDown { x, y }, View::Note(_))
                if !self.menu && (TOP..TOP + self.view_rows() * ROW).contains(&y) && PAPER.contains_point(x, y) =>
            {
                match id {
                    Some(id) => self.pen_down(cx, id, x, y),
                    None => Reply::Nothing,
                }
            }
            (Event::PenMove { x, y }, View::Note(_)) if self.inking || self.tap.is_some() => self.pen_move(cx, x, y),
            (Event::PenUp { x, y }, View::Note(_)) if self.inking || self.tap.is_some() => match id {
                Some(id) => self.pen_up(cx, id, x, y),
                None => Reply::Nothing,
            },
            (Event::Char(c), View::Note(_)) if self.typing => {
                if let Some(id) = id {
                    self.type_char(cx, id, c);
                    self.redo_n = 0;
                }
                Reply::Redraw
            }
            (Event::Suggestion(k), View::Note(_)) if self.typing => {
                if let Some(id) = id {
                    self.take_suggestion(cx, id, usize::from(k));
                }
                Reply::Redraw
            }
            (Event::Delete, View::Note(_)) if self.typing => {
                if let Some(id) = id {
                    self.untype(cx, id);
                    self.follow = true;
                }
                Reply::Redraw
            }
            (Event::Done, View::Note(_)) if self.typing => {
                if let Some(id) = id {
                    self.new_line(cx, id);
                }
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    /// Note `id` taken out of the store, its ink pages and text pages with
    /// it.
    fn delete_note(&mut self, cx: &Ctx, id: u32) {
        if let Some(cell) = cx.pim
            && let Ok(mut g) = cell.try_borrow_mut()
            && let Some(store) = g.as_mut()
        {
            let (page, np) = pages(store, id);
            for &p in &page[..np] {
                let _ = store.remove(p);
            }
            doc::remove_pages(store, id);
            let _ = store.remove(id);
        }
    }

    /// What a stylus can tap in an open note besides writing, for the
    /// D-pad: the menu's keys while it is up; else the tick boxes, the
    /// colour wells, the sizes and the tools.
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
            let k = match self.menu_row {
                1 => 5,
                2 => 1,
                _ => 6,
            };
            for i in 0..k {
                add(menu_key(i, k));
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
            for k in 0..TOOLS.len() {
                add(tool_key(k));
            }
        }
        (t, n)
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_d;
