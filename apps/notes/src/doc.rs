//! A note's text of any length (notes-rule0.md, "Notes of any length").
//!
//! Iterate, don't imitate (rule 0): Windows Mobile kept a note as one file
//! rewritten whole on every save; version 1 kept it in one store field and
//! stopped at 2,000 bytes. A modern editor keeps a rope or a piece table in
//! RAM. Ours keeps the note's lines in **chunks**: the note's own `TEXT`
//! first, then text pages (`Kind::Ink` records holding `TEXT` and
//! [`tag::ORDER`], named by `PARENT`), each at most [`TEXT_MAX`] bytes of
//! whole lines. An edit copies and rewrites only the chunk holding its
//! line, so a keystroke in a 100 KB note appends one chunk to the store's
//! log, never the note; a chunk that grows past its room splits at the line
//! nearest its middle (a line longer than a chunk goes on in a line marked
//! [`CONT`]). A note's length is bounded only by the store's room.
//!
//! The lines: text, `[ ] `/`[x] ` checklist items, and ink lines
//! ([`INK_LINE`], then the ink block's number as a byte `0x20 + n` for
//! blocks after the first: the block's strokes carry the same number).

use rp_pim::store::{Kind, Rec, Store, TEXT_MAX, tag};

/// The line standing for an ink block.
pub const INK_LINE: u8 = 0x01;
/// A line's first byte when it continues the line before it (a paragraph
/// longer than a chunk): drawn and exported as one paragraph.
pub const CONT: u8 = 0x02;
/// Most chunks a note's text is read through (128 x 2,000 bytes is the
/// store's whole room).
pub const MAX_CHUNKS: usize = 128;
/// A new page's [`tag::ORDER`] step: room for 65,535 pages split in
/// between before the pages are numbered again.
const STEP: u32 = 1 << 16;
/// The room an edit may add to a chunk at once (a line put in, a pasted
/// clipboard): the edit's buffer is a chunk and this.
pub const EDIT_ROOM: usize = 1024;

/// Whether `r` is a page of a note's text (not of its ink).
#[must_use]
pub fn is_text_page(r: &Rec<'_>) -> bool {
    r.kind == Kind::Ink && r.num(tag::ORDER).is_some()
}

/// The ink block an ink line names, or `None` for any other line.
#[must_use]
pub fn ink_block(line: &[u8]) -> Option<u8> {
    match line {
        [INK_LINE] => Some(0),
        [INK_LINE, b] if (0x21..0x7F).contains(b) => Some(b - 0x20),
        _ => None,
    }
}

/// The line standing for ink block `b` (1..=94 after the first).
#[must_use]
pub fn ink_line(b: u8) -> ([u8; 2], usize) {
    if b == 0 {
        ([INK_LINE, 0], 1)
    } else {
        ([INK_LINE, 0x20 + b.min(94)], 2)
    }
}

/// Note `id`'s chunks in order: the note's own record, then its text pages
/// by [`tag::ORDER`] (then id).
#[must_use]
pub fn chunks(store: &Store<'_>, id: u32) -> ([u32; MAX_CHUNKS], usize) {
    let mut ids = [0u32; MAX_CHUNKS];
    let mut keys = [(0u32, 0u32); MAX_CHUNKS];
    ids[0] = id;
    let mut n = 1;
    for r in store.all(Kind::Ink) {
        if r.num(tag::PARENT) == Some(id) && n < MAX_CHUNKS {
            let Some(order) = r.num(tag::ORDER) else {
                continue;
            };
            let key = (order, r.id);
            let at = (1..n).find(|&k| keys[k] > key).unwrap_or(n);
            keys.copy_within(at..n, at + 1);
            ids.copy_within(at..n, at + 1);
            keys[at] = key;
            ids[at] = r.id;
            n += 1;
        }
    }
    (ids, n)
}

/// The lines of one chunk's text (none for an empty one).
fn lines_in(text: &[u8]) -> usize {
    if text.is_empty() {
        0
    } else {
        text.iter().filter(|&&c| c == b'\n').count() + 1
    }
}

/// Every line of note `id` given to `f` with its number, in order, until
/// `f` answers false. The lines read.
pub fn for_lines(store: &Store<'_>, id: u32, f: &mut dyn FnMut(usize, &[u8]) -> bool) -> usize {
    let (ids, n) = chunks(store, id);
    let mut k = 0;
    for &c in &ids[..n] {
        let Some(r) = store.get(c) else { continue };
        let t = r.text(tag::TEXT);
        if t.is_empty() {
            continue;
        }
        for l in t.split(|&c| c == b'\n') {
            if !f(k, l) {
                return k + 1;
            }
            k += 1;
        }
    }
    k
}

/// How many lines note `id` has.
#[must_use]
pub fn line_count(store: &Store<'_>, id: u32) -> usize {
    for_lines(store, id, &mut |_, _| true)
}

/// Line `k` of note `id` copied into `out` (cut to it): its length.
pub fn line(store: &Store<'_>, id: u32, k: usize, out: &mut [u8]) -> Option<usize> {
    let mut got = None;
    for_lines(store, id, &mut |i, l| {
        if i == k {
            let n = l.len().min(out.len());
            out[..n].copy_from_slice(&l[..n]);
            got = Some(n);
            false
        } else {
            true
        }
    });
    got
}

/// The last `out.len()` bytes of line `k` (the caret is at a line's end:
/// the word being typed and the sentence it is in): how many, and whether
/// that is the whole line.
pub fn line_tail(store: &Store<'_>, id: u32, k: usize, out: &mut [u8]) -> Option<(usize, bool)> {
    let mut got = None;
    for_lines(store, id, &mut |i, l| {
        if i == k {
            let n = l.len().min(out.len());
            out[..n].copy_from_slice(&l[l.len() - n..]);
            got = Some((n, n == l.len()));
            false
        } else {
            true
        }
    });
    got
}

/// Where line `k` lies: its chunk's place in [`chunks`], the chunk's id,
/// and the line's start in the chunk's text. For `k` one past the last
/// line, the end of the last chunk holding text (or the note's own).
fn find(store: &Store<'_>, id: u32, k: usize) -> Option<(usize, u32, usize, bool)> {
    let (ids, n) = chunks(store, id);
    let mut first = 0;
    let mut last = (0usize, id, 0usize);
    for (place, &c) in ids[..n].iter().enumerate() {
        let r = store.get(c)?;
        let t = r.text(tag::TEXT);
        let lines = lines_in(t);
        if k < first + lines {
            let mut at = 0;
            for _ in first..k {
                at += t[at..].iter().position(|&b| b == b'\n')? + 1;
            }
            return Some((place, c, at, false));
        }
        first += lines;
        if lines > 0 {
            last = (place, c, t.len());
        }
    }
    (k == first).then_some((last.0, last.1, last.2, true))
}

/// What an edit does to a chunk's text held in a buffer.
pub enum Edit<'a> {
    /// `bytes` put at the end of line `k`.
    Append(usize, &'a [u8]),
    /// The last character (a whole UTF-8 sequence) of line `k` taken off:
    /// its bytes into the given buffer.
    Pop(usize),
    /// A new line holding `bytes` put in before line `k` (`k` the count:
    /// at the end).
    Insert(usize, &'a [u8]),
    /// Line `k` taken out.
    Remove(usize),
    /// Line `k` replaced by `bytes`.
    Set(usize, &'a [u8]),
    /// A new last line holding `bytes` (no line counted: an import's
    /// thousands of lines each cost one chunk, not the note).
    Push(&'a [u8]),
    /// The last line taken out (an import taken back).
    DropLast,
    /// Checklist item `k` ticked or unticked (`[ ] ` and `[x] `).
    Tick(usize),
}

/// Why an edit didn't apply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refused {
    /// No such line, or nothing to take.
    NoLine,
    /// The store is full (or busy): nothing changed.
    Full,
}

/// One edit to note `id`'s text, its chunk rewritten (split where it grew
/// past its room). For [`Edit::Pop`], the bytes taken (at most 4).
///
/// # Errors
/// [`Refused`].
#[inline(never)]
pub fn edit(store: &mut Store<'_>, id: u32, e: &Edit<'_>) -> Result<([u8; 4], usize), Refused> {
    // Typing at a paragraph goes to its end: the last of the continuation
    // lines it was cut into (an emptied continuation goes, and the
    // character before it is the one taken).
    let e = &match *e {
        Edit::Append(k, b) => Edit::Append(para_end(store, id, k), b),
        Edit::Pop(k) => {
            let end = para_end(store, id, k);
            if end > k {
                let mut one = [0u8; 2];
                if line(store, id, end, &mut one) == Some(1) {
                    edit(store, id, &Edit::Remove(end))?;
                    return edit(store, id, &Edit::Pop(k));
                }
            }
            Edit::Pop(end)
        }
        ref other => match other {
            Edit::Insert(k, b) => Edit::Insert(*k, b),
            Edit::Remove(k) => Edit::Remove(*k),
            Edit::Set(k, b) => Edit::Set(*k, b),
            Edit::Push(b) => Edit::Push(b),
            Edit::Tick(k) => Edit::Tick(*k),
            _ => Edit::DropLast,
        },
    };
    let (place, rec, at, end) = match e {
        Edit::Append(k, _)
        | Edit::Pop(k)
        | Edit::Remove(k)
        | Edit::Set(k, _)
        | Edit::Insert(k, _)
        | Edit::Tick(k) => find(store, id, *k).ok_or(Refused::NoLine)?,
        Edit::Push(_) | Edit::DropLast => {
            let (ids, n) = chunks(store, id);
            let (place, c) = (0..n)
                .rev()
                .map(|p| (p, ids[p]))
                .find(|&(_, c)| store.get(c).is_some_and(|r| !r.text(tag::TEXT).is_empty()))
                .unwrap_or((0, id));
            let t = store.get(c).map_or(&[][..], |r| r.text(tag::TEXT));
            if matches!(e, Edit::DropLast) {
                if t.is_empty() {
                    return Err(Refused::NoLine);
                }
                let at = t.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
                (place, c, at, false)
            } else {
                (place, c, t.len(), true)
            }
        }
    };
    if end && !matches!(e, Edit::Insert(..) | Edit::Push(_)) {
        return Err(Refused::NoLine);
    }
    let mut buf = [0u8; TEXT_MAX + EDIT_ROOM];
    let r = store.get(rec).ok_or(Refused::NoLine)?;
    let mut n = r.text(tag::TEXT).len().min(TEXT_MAX);
    buf[..n].copy_from_slice(&r.text(tag::TEXT)[..n]);
    let line_end = |buf: &[u8], n: usize| {
        at + buf[at..n]
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(n - at)
    };
    let mut taken = ([0u8; 4], 0usize);
    let put = |buf: &mut [u8], n: &mut usize, pos: usize, bytes: &[u8]| -> Result<(), Refused> {
        if *n + bytes.len() > buf.len() {
            return Err(Refused::Full);
        }
        buf.copy_within(pos..*n, pos + bytes.len());
        buf[pos..pos + bytes.len()].copy_from_slice(bytes);
        *n += bytes.len();
        Ok(())
    };
    match *e {
        Edit::Append(_, bytes) => {
            let le = line_end(&buf, n);
            put(&mut buf, &mut n, le, bytes)?;
        }
        Edit::Pop(_) => {
            let le = line_end(&buf, n);
            if le == at {
                return Err(Refused::NoLine);
            }
            // A whole UTF-8 sequence: continuation bytes with their lead.
            let mut s = le - 1;
            while s > at && le - s < 4 && buf[s] & 0xC0 == 0x80 {
                s -= 1;
            }
            taken.1 = le - s;
            taken.0[..taken.1].copy_from_slice(&buf[s..le]);
            buf.copy_within(le..n, s);
            n -= le - s;
        }
        Edit::Insert(_, bytes) | Edit::Push(bytes) => {
            if end {
                // After the last line (or into an empty chunk).
                if n > 0 {
                    put(&mut buf, &mut n, at, b"\n")?;
                    let at = at + 1;
                    put(&mut buf, &mut n, at, bytes)?;
                } else {
                    put(&mut buf, &mut n, 0, bytes)?;
                }
            } else {
                put(&mut buf, &mut n, at, b"\n")?;
                put(&mut buf, &mut n, at, bytes)?;
            }
        }
        Edit::Tick(_) => match buf.get(at..at + 4) {
            Some(b"[ ] ") => buf[at + 1] = b'x',
            Some(b"[x] ") => buf[at + 1] = b' ',
            _ => return Err(Refused::NoLine),
        },
        Edit::Remove(_) | Edit::DropLast => {
            let le = line_end(&buf, n);
            // The line and its newline (the one before it for the last).
            let (s, e2) = if le < n {
                (at, le + 1)
            } else if at > 0 {
                (at - 1, le)
            } else {
                (0, le)
            };
            buf.copy_within(e2..n, s);
            n -= e2 - s;
        }
        Edit::Set(_, bytes) => {
            let le = line_end(&buf, n);
            buf.copy_within(le..n, at);
            n -= le - at;
            put(&mut buf, &mut n, at, bytes)?;
        }
    }
    write_chunk(store, id, place, rec, &mut buf, n)?;
    Ok(taken)
}

/// A chunk's new text written back: as it is where it fits, an emptied
/// page removed, else split in two at the line nearest its middle (a
/// single line too long goes on in a [`CONT`] line).
fn write_chunk(
    store: &mut Store<'_>,
    note: u32,
    place: usize,
    rec: u32,
    buf: &mut [u8],
    n: usize,
) -> Result<(), Refused> {
    let page = rec != note;
    if page && n == 0 {
        return store.remove(rec).map_err(|_| Refused::Full);
    }
    if n <= TEXT_MAX && store.change(rec, &[(tag::TEXT, &buf[..n])]).is_ok() {
        return Ok(());
    }
    if !page && n <= TEXT_MAX && n > 0 && ink_more_out(store, note) {
        // The note's record was full of ink: its second ink field went to
        // a page of its own, and the text fits beside the first.
        if store
            .change(rec, &[(tag::TEXT, &buf[..n]), (tag::INK_MORE, &[])])
            .is_ok()
        {
            return Ok(());
        }
    }
    // Split once, in place (no second buffer: programs run on the image's
    // 24 KiB stack): the first part stays, the rest goes to a new page
    // after it, both within a chunk's room. The cut: the line end nearest
    // the middle that leaves both parts in room; else (lines longer than
    // that) a character boundary there, the rest a continuation line.
    let mid = n / 2;
    let lo = n.saturating_sub(TEXT_MAX - 1).max(1);
    let hi = TEXT_MAX.min(n.saturating_sub(1));
    let cut = buf[..n]
        .iter()
        .enumerate()
        .filter(|&(i, &b)| b == b'\n' && (lo..=hi).contains(&i))
        .map(|(i, _)| i)
        .min_by_key(|&i| i.abs_diff(mid));
    let (first, rest_from, cont) = match cut {
        Some(i) => (i, i + 1, false),
        None => {
            let mut i = mid.clamp(lo, hi.max(lo));
            while i > lo && buf[i] & 0xC0 == 0x80 {
                i -= 1;
            }
            (i, i, true)
        }
    };
    // The new page's order: between this chunk and the next.
    let order = order_after(store, note, place)?;
    let o = order.to_le_bytes();
    let p = note.to_le_bytes();
    // A continuation's mark goes in the byte before the rest for the page's
    // write (that byte put back after: it is the first part's last).
    let from = if cont { rest_from - 1 } else { rest_from };
    let kept = buf[from];
    if cont {
        buf[from] = CONT;
    }
    // The new page first (holding what would be lost), then the old chunk
    // cut down: a failure between leaves a line twice, never none.
    let put = store.put(
        Kind::Ink,
        None,
        &[
            (tag::PARENT, &p),
            (tag::ORDER, &o),
            (tag::TEXT, &buf[from..n]),
        ],
    );
    buf[from] = kept;
    let new = put.map_err(|_| Refused::Full)?;
    if let Err(e) = write_chunk(store, note, place, rec, buf, first) {
        let _ = store.remove(new);
        return Err(e);
    }
    Ok(())
}

/// The note's second ink field moved to an ink page of its own.
fn ink_more_out(store: &mut Store<'_>, id: u32) -> bool {
    let mut ink = [0u8; TEXT_MAX];
    let Some(r) = store.get(id) else { return false };
    let n = r.text(tag::INK_MORE).len().min(ink.len());
    if n == 0 {
        return false;
    }
    ink[..n].copy_from_slice(&r.text(tag::INK_MORE)[..n]);
    store
        .put(
            Kind::Ink,
            None,
            &[(tag::PARENT, &id.to_le_bytes()), (tag::INK, &ink[..n])],
        )
        .is_ok()
}

/// The order for a new page right after chunk `place` (the pages numbered
/// again first where no gap is left).
fn order_after(store: &mut Store<'_>, note: u32, place: usize) -> Result<u32, Refused> {
    let order_of = |store: &Store<'_>, c: u32| -> u32 {
        if c == note {
            0
        } else {
            store.get(c).and_then(|r| r.num(tag::ORDER)).unwrap_or(0)
        }
    };
    let (ids, n) = chunks(store, note);
    let a = order_of(store, ids[place]);
    let b = if place + 1 < n {
        order_of(store, ids[place + 1])
    } else {
        a.saturating_add(2 * STEP)
    };
    if b > a + 1 {
        return Ok(a + (b - a) / 2);
    }
    // No gap: every page numbered again, STEP apart.
    for (k, &c) in ids[1..n].iter().enumerate() {
        let o = ((k as u32 + 1) * STEP).to_le_bytes();
        store
            .change(c, &[(tag::ORDER, &o)])
            .map_err(|_| Refused::Full)?;
    }
    Ok((place as u32) * STEP + STEP / 2)
}

/// The last line of the paragraph line `k` begins: `k`, or the last of the
/// continuation lines that follow it.
fn para_end(store: &Store<'_>, id: u32, k: usize) -> usize {
    let mut end = k;
    for_lines(store, id, &mut |i, l| {
        if i <= k {
            return true;
        }
        if l.first() == Some(&CONT) {
            end = i;
            true
        } else {
            false
        }
    });
    end
}

/// Every text page of note `id` taken out (the note's own record is the
/// caller's).
pub fn remove_pages(store: &mut Store<'_>, id: u32) {
    let (ids, n) = chunks(store, id);
    for &c in &ids[1..n] {
        let _ = store.remove(c);
    }
}

/// Whether any line of note `id` holds `q`, whatever its capitals.
#[must_use]
pub fn holds(store: &Store<'_>, id: u32, q: &[u8]) -> bool {
    let mut found = false;
    for_lines(store, id, &mut |_, l| {
        found = l.len() >= q.len() && l.windows(q.len()).any(|w| w.eq_ignore_ascii_case(q));
        !found
    });
    found
}
