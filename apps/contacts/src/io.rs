//! Contacts' files, duplicates and pictures (contacts-rule0.md "vCard 3
//! and 4 import and export", "Duplicates offered as one merge", "Photos
//! from a picture file on the card, and from vCard PHOTO").
//!
//! Iterate, don't imitate (rule 0): Windows Mobile read a vCard only
//! through ActiveSync, one at a time; Google imports a file and leaves the
//! doubles for a "merge duplicates" pass the user must remember; iOS
//! merges silently and sometimes wrongly. Ours lists the card's and the
//! PC's .vcf files together (the card needs no dock), imports every person
//! with their picture (decoded as it is read, never held whole), and then
//! **offers each double as one merge at once**: the two side by side, what
//! they share, what the merged card keeps; "Keep both" is remembered so a
//! pair is never asked twice. Export writes every person as vCard 3.0
//! (what Outlook and every phone read) or 4.0, a piece at a time. A
//! picture file on the card is read in pieces a camera's 4 MB needs (its
//! own thumbnail first: rp_pim::jpeg).

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Event, Frame, Key, MARGIN, Pic, Rect, Reply, Text, WIDE};
use rp_pim::jpeg::{self, Decoder, Progress};
use rp_pim::store::{Kind, Store, tag};
use rp_pim::vcard::{self, Card, Version};

use super::{Contacts, PHOTO_PAGES, View, photo};

/// Where a file comes from or goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum From {
    /// The device's own card.
    Card,
    /// The PC on the dock.
    Pc,
}

/// A question about files, to the card or the PC.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    /// The .vcf files there.
    List,
    /// File `n` of the list.
    File(u8),
    /// Every person written there.
    Put,
    /// The card's pictures.
    Pics,
    /// Picture `n` of the card's from byte `at`.
    Pic(u8, u64),
}

/// The tools' keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Import,
    ToCard,
    ToPc,
    V3,
    V4,
    Doubles,
}

/// The export under way: its length, how far it has been sent (the
/// record and the byte it starts at), the store's change count it was
/// counted at, the day its name carries.
#[derive(Clone, Copy, Debug)]
struct Export {
    len: usize,
    rec: usize,
    rec_at: usize,
    changes: u32,
    day: i32,
}

/// A card picture being read into a person's: the person, the card's
/// file and how far into it, the picture slot the square adds up in.
#[derive(Clone, Copy, Debug)]
struct PicJob {
    id: u32,
    n: u8,
    pic: Pic,
}

/// The files' rows a screen lists.
const ROWS: usize = 10;
/// Doubles found at most.
const PAIRS: usize = 12;
/// The pairs "Keep both" remembers (a `Kind::Data` record "apart").
const APART: usize = 64;
/// The key under the list's search field that opens the tools.
pub const TOOLS_KEY: Rect = Rect::new(MARGIN + WIDE - 52, 44, 52, 40);
/// The merge screen's keys.
const MERGE_KEY: Rect = Rect::new(MARGIN, 532, WIDE / 2 - 4, 52);
const APART_KEY: Rect = Rect::new(MARGIN + WIDE / 2 + 4, 532, WIDE / 2 - 4, 52);

/// The tools', the files' and the merge screen's state.
pub struct Io {
    rows: [(From, u8, Text<40>); ROWS],
    n_rows: usize,
    rows_at: [(Rect, u8); ROWS],
    n_at: usize,
    tools_at: [(Rect, Tool); 6],
    n_tools: usize,
    want_card: Option<Want>,
    want_pc: Option<Want>,
    card_asked: Option<Want>,
    pc_asked: Option<Want>,
    chosen: Text<40>,
    export: Option<Export>,
    /// Export as vCard 4.0 (3.0 when false).
    pub v4: bool,
    /// What the last import, export or merge did.
    pub status: Text<48>,
    pairs: [(u32, u32); PAIRS],
    pub(crate) n_pairs: usize,
    /// The card's pictures for the chooser (their numbers and names).
    pub(crate) card_pics: [(u8, Text<28>); 8],
    pub(crate) n_card_pics: usize,
    job: Option<PicJob>,
}

impl Io {
    /// Nothing under way.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rows: [(From::Card, 0, Text::EMPTY); ROWS],
            n_rows: 0,
            rows_at: [(Rect::new(0, 0, 0, 0), 0); ROWS],
            n_at: 0,
            tools_at: [(Rect::new(0, 0, 0, 0), Tool::Import); 6],
            n_tools: 0,
            want_card: None,
            want_pc: None,
            card_asked: None,
            pc_asked: None,
            chosen: Text::EMPTY,
            export: None,
            v4: false,
            status: Text::EMPTY,
            pairs: [(0, 0); PAIRS],
            n_pairs: 0,
            card_pics: [(0, Text::EMPTY); 8],
            n_card_pics: 0,
            job: None,
        }
    }

    /// Whether the answer coming is the files' (the card's, or the PC's).
    #[must_use]
    pub const fn asking(&self) -> bool {
        self.card_asked.is_some() || self.pc_asked.is_some()
    }

    /// The rows a stylus or the D-pad can choose on the screen shown.
    pub fn targets(&self, view: View) -> impl Iterator<Item = Rect> + '_ {
        let tools = view == View::Tools;
        let files = view == View::Files;
        let merge = view == View::Merge;
        self.tools_at[..self.n_tools]
            .iter()
            .map(|t| t.0)
            .filter(move |_| tools)
            .chain(
                self.rows_at[..self.n_at]
                    .iter()
                    .map(|r| r.0)
                    .filter(move |_| files),
            )
            .chain([MERGE_KEY, APART_KEY].into_iter().filter(move |_| merge))
    }
}

impl Default for Io {
    fn default() -> Self {
        Self::new()
    }
}

fn digits(v: usize, out: &mut [u8; 8]) -> usize {
    let mut d = [0u8; 8];
    let mut k = d.len();
    let mut v = v;
    loop {
        k -= 1;
        d[k] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 || k == 0 {
            break;
        }
    }
    let n = d.len() - k;
    out[..n].copy_from_slice(&d[k..]);
    n
}

/// Text made of parts (no allocation).
fn join<const N: usize>(parts: &[&[u8]]) -> Text<N> {
    let mut b = [0u8; 64];
    let mut n = 0;
    for p in parts {
        for &c in *p {
            if n < b.len() {
                b[n] = c;
                n += 1;
            }
        }
    }
    Text::of(&b[..n])
}

/// "Contacts 2026-11-23.vcf".
fn export_name(day: i32) -> Text<40> {
    let d = rp_pim::date::date_of(day);
    let two = |v: u8| [b'0' + v / 10, b'0' + v % 10];
    let mut y = [0u8; 8];
    let n = digits(d.year.unsigned_abs() as usize, &mut y);
    join(&[
        b"Contacts ",
        &y[..n],
        b"-",
        &two(d.month),
        b"-",
        &two(d.day),
        b".vcf",
    ])
}

/// The pairs "Keep both" was answered for.
fn apart(store: &Store<'_>) -> ([(u32, u32); APART], usize, Option<u32>) {
    let mut out = [(0u32, 0u32); APART];
    let mut n = 0;
    let rec = store
        .all(Kind::Data)
        .find(|r| r.text(tag::NAME) == b"apart");
    if let Some(r) = &rec {
        for p in r.text(tag::TEXT).as_chunks::<8>().0 {
            if n < APART {
                out[n] = (
                    u32::from_le_bytes([p[0], p[1], p[2], p[3]]),
                    u32::from_le_bytes([p[4], p[5], p[6], p[7]]),
                );
                n += 1;
            }
        }
    }
    (out, n, rec.map(|r| r.id))
}

/// Every pair of people with a number or an e-mail in common (lower id
/// first), but those "Keep both" was answered for.
fn doubles(store: &Store<'_>, out: &mut [(u32, u32); PAIRS]) -> usize {
    let (apart, n_apart, _) = apart(store);
    let mut n = 0;
    for a in store.all(Kind::Contact) {
        let mut ka = [0u64; 16];
        let na = rp_pim::contact::keys(&a, &mut ka);
        if na == 0 {
            continue;
        }
        for b in store.all(Kind::Contact).filter(|b| b.id > a.id) {
            let mut kb = [0u64; 16];
            let nb = rp_pim::contact::keys(&b, &mut kb);
            if ka[..na].iter().any(|k| kb[..nb].contains(k))
                && !apart[..n_apart].contains(&(a.id, b.id))
                && n < PAIRS
            {
                out[n] = (a.id, b.id);
                n += 1;
            }
        }
    }
    n
}

impl Contacts {
    // ---- The tools ---------------------------------------------------

    /// The list's key that opens the tools: three dots on an Aero key.
    pub(crate) fn tools_key(&self, f: &mut Frame, cx: &Ctx) {
        kit::key(f, cx, TOOLS_KEY, kit::KeyState::Rest);
        for k in 0..3u16 {
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(TOOLS_KEY.x + 16 + k * 8, TOOLS_KEY.y + 18, 4, 4),
                color: TEXT,
            });
        }
    }

    /// The tools opened: the doubles counted afresh.
    pub(crate) fn open_tools(&mut self, cx: &Ctx) {
        self.go(View::Tools);
        self.count_doubles(cx);
    }

    /// The doubles counted from the store.
    pub(crate) fn count_doubles(&mut self, cx: &Ctx) {
        if let Some(g) = cx.pim.and_then(|c| c.try_borrow().ok())
            && let Some(s) = g.as_ref()
        {
            self.io.n_pairs = doubles(s, &mut self.io.pairs);
        }
    }

    /// The import's files asked for, as its key does (the previews).
    pub fn ask_import(&mut self) {
        self.io.n_rows = 0;
        self.io.want_card = Some(Want::List);
        self.io.want_pc = Some(Want::List);
        self.io.status = Text::of(b"Looking on the card and your PC...");
        self.go(View::Files);
    }

    pub(crate) fn tools_view(&mut self, f: &mut Frame, cx: &Ctx) {
        self.io.n_tools = 0;
        let mut y = kit::large_title(
            f,
            cx,
            36,
            b"Import and export",
            b"vCard files on the card or your PC",
            tint::CONTACTS,
        ) + 8;
        let add = |this: &mut Self,
                   f: &mut Frame,
                   r: Rect,
                   t: Tool,
                   word: &[u8],
                   sub: &[u8],
                   on: bool| {
            kit::key(
                f,
                cx,
                r,
                if on {
                    kit::KeyState::On
                } else {
                    kit::KeyState::Rest
                },
            );
            kit::text(f, r.x + 16, r.y + 24, cx.fonts.ui, TEXT, word);
            kit::fitted(f, r.x + 16, r.y + 44, cx.fonts.label, DIM, sub, r.w - 32);
            this.io.tools_at[this.io.n_tools] = (r, t);
            this.io.n_tools += 1;
        };
        let name = export_name(cx.now.day);
        add(
            self,
            f,
            Rect::new(MARGIN, y, WIDE, 52),
            Tool::Import,
            b"Import contacts",
            b"A .vcf file on the card or your PC (vCard 2.1, 3.0, 4.0)",
            false,
        );
        y += 64;
        kit::section(f, cx, y, b"EXPORT EVERYONE", tint::CONTACTS);
        y += 22;
        add(
            self,
            f,
            Rect::new(MARGIN, y, WIDE, 52),
            Tool::ToCard,
            b"Save to the card",
            join::<64>(&[b"RetroPane\\", name.get()]).get(),
            false,
        );
        y += 58;
        add(
            self,
            f,
            Rect::new(MARGIN, y, WIDE, 52),
            Tool::ToPc,
            b"Save to your PC",
            b"Documents\\RetroPane, over the dock",
            false,
        );
        y += 60;
        // The format: 3.0 by default (Outlook reads no 4.0).
        kit::text(f, MARGIN + 6, y + 26, cx.fonts.label, DIM, b"FORMAT");
        for (k, (t, word)) in [(Tool::V3, &b"vCard 3.0"[..]), (Tool::V4, b"vCard 4.0")]
            .into_iter()
            .enumerate()
        {
            let r = Rect::new(MARGIN + 96 + k as u16 * 150, y + 2, 140, 38);
            let on = (t == Tool::V4) == self.io.v4;
            kit::key(
                f,
                cx,
                r,
                if on {
                    kit::KeyState::On
                } else {
                    kit::KeyState::Rest
                },
            );
            kit::centred(f, r, r.y + 25, cx.fonts.ui, TEXT, word);
            self.io.tools_at[self.io.n_tools] = (r, t);
            self.io.n_tools += 1;
        }
        y += 56;
        kit::section(f, cx, y, b"DUPLICATES", tint::CONTACTS);
        y += 22;
        let mut count = [0u8; 8];
        let cn = digits(self.io.n_pairs, &mut count);
        let line: Text<48> = if self.io.n_pairs == 0 {
            join(&[b"No one is here twice"])
        } else {
            join(&[
                &count[..cn],
                if self.io.n_pairs == 1 {
                    b" person may be here twice"
                } else {
                    b" people may be here twice"
                },
            ])
        };
        let on = self.io.n_pairs > 0;
        add(
            self,
            f,
            Rect::new(MARGIN, y, WIDE, 52),
            Tool::Doubles,
            line.get(),
            if on {
                b"The same number or e-mail: see each as one"
            } else {
                b"Found by a shared number or e-mail"
            },
            on,
        );
        y += 64;
        if !self.io.status.is_empty() {
            kit::text(
                f,
                MARGIN + 6,
                y + 18,
                cx.fonts.ui,
                kit::FOCUS,
                self.io.status.get(),
            );
        }
    }

    // ---- The files' screen ---------------------------------------------

    pub(crate) fn files_view(&mut self, f: &mut Frame, cx: &Ctx) {
        self.io.n_at = 0;
        let mut y = kit::large_title(
            f,
            cx,
            36,
            b"Import contacts",
            b".vcf files on the card and your PC",
            tint::CONTACTS,
        ) + 8;
        for k in 0..self.io.n_rows {
            let (from, _, name) = self.io.rows[k];
            let r = Rect::new(MARGIN, y, WIDE, 46);
            kit::panel(f, cx, r);
            kit::fitted(
                f,
                r.x + 14,
                r.y + 29,
                cx.fonts.ui,
                TEXT,
                name.get(),
                r.w - 110,
            );
            kit::right(
                f,
                r.right() - 14,
                r.y + 29,
                cx.fonts.label,
                tint::CONTACTS,
                match from {
                    From::Card => b"CARD",
                    From::Pc => b"PC",
                },
            );
            self.io.rows_at[self.io.n_at] = (r, k as u8);
            self.io.n_at += 1;
            y += 50;
        }
        if self.io.n_rows == 0 && self.io.want_pc.is_none() && self.io.pc_asked.is_none() {
            for (k, l) in [
                &b"No .vcf file on the card or the PC yet."[..],
                b"Put one in the card's RetroPane folder,",
                b"or in Documents\\RetroPane on your PC.",
            ]
            .iter()
            .enumerate()
            {
                kit::text(f, MARGIN + 8, y + 26 + k as u16 * 24, cx.fonts.ui, DIM, l);
            }
            y += 80;
        }
        if !self.io.status.is_empty() {
            kit::text(
                f,
                MARGIN + 8,
                y + 26,
                cx.fonts.ui,
                kit::FOCUS,
                self.io.status.get(),
            );
        }
    }

    // ---- Events on the new screens -----------------------------------

    pub(crate) fn io_event(&mut self, e: Event, cx: &Ctx) -> Reply {
        let Event::PenUp { x, y } = e else {
            if e == Event::Key(Key::SoftLeft) {
                match self.view {
                    View::Tools => self.go(View::List),
                    _ => self.open_tools(cx),
                }
                return Reply::Redraw;
            }
            return Reply::Nothing;
        };
        match self.view {
            View::Tools => {
                let Some(&(_, t)) = self.io.tools_at[..self.io.n_tools]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                else {
                    return Reply::Nothing;
                };
                match t {
                    Tool::Import => self.ask_import(),
                    Tool::ToCard => self.start_export(cx, From::Card),
                    Tool::ToPc => self.start_export(cx, From::Pc),
                    Tool::V3 => self.io.v4 = false,
                    Tool::V4 => self.io.v4 = true,
                    Tool::Doubles if self.io.n_pairs > 0 => self.go(View::Merge),
                    Tool::Doubles => return Reply::Nothing,
                }
                Reply::Redraw
            }
            View::Files => {
                let Some(&(_, k)) = self.io.rows_at[..self.io.n_at]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                else {
                    return Reply::Nothing;
                };
                let (from, n, name) = self.io.rows[usize::from(k)];
                self.io.chosen = name;
                match from {
                    From::Card => self.io.want_card = Some(Want::File(n)),
                    From::Pc => self.io.want_pc = Some(Want::File(n)),
                }
                self.io.status = Text::of(b"Reading the file...");
                Reply::Redraw
            }
            View::Merge => {
                if MERGE_KEY.contains_point(x, y) {
                    self.merge_pair(cx);
                } else if APART_KEY.contains_point(x, y) {
                    self.keep_apart(cx);
                } else {
                    return Reply::Nothing;
                }
                self.next_pair(cx);
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    // ---- The questions -----------------------------------------------

    /// The card's question, if one waits.
    pub(crate) fn io_card_ask(&mut self) -> Option<rp_app::NetAsk> {
        let want = self.io.want_card.take()?;
        self.io.card_asked = Some(want);
        Some(self.io_ask(want, true))
    }

    /// The PC's question, if one waits.
    pub(crate) fn io_net_ask(&mut self) -> Option<rp_app::NetAsk> {
        let want = self.io.want_pc.take()?;
        self.io.pc_asked = Some(want);
        Some(self.io_ask(want, false))
    }

    fn io_ask(&self, want: Want, card: bool) -> rp_app::NetAsk {
        let mut n = [0u8; 8];
        let (what, arg): (&'static [u8], Text<64>) = match (want, card) {
            (Want::List, true) => (b"CARDFILES", Text::of(b"vcf")),
            (Want::List, false) => (b"FILES", Text::of(b"vcf")),
            (Want::Pics, _) => (b"CARDFILES", Text::of(b"pic")),
            (Want::File(k), c) => {
                let l = digits(usize::from(k), &mut n);
                (
                    if c { b"CARDFILE" } else { b"FILE" },
                    join(&[&n[..l], b" vcf"]),
                )
            }
            (Want::Pic(k, at), _) => {
                let l = digits(usize::from(k), &mut n);
                let mut a = [0u8; 8];
                let al = digits(at as usize, &mut a);
                (b"CARDFILE", join(&[&n[..l], b" pic ", &a[..al]]))
            }
            (Want::Put, c) => {
                let x = self.io.export.map_or((0, 0), |x| (x.len, x.day));
                let l = digits(x.0, &mut n);
                (
                    if c { b"CARDPUT" } else { b"PUT" },
                    join(&[&n[..l], b" ", export_name(x.1).get()]),
                )
            }
        };
        rp_app::NetAsk { what, arg }
    }

    /// An answer to a files' question.
    pub(crate) fn io_answer(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
        let (want, from) = match (self.io.card_asked.take(), self.io.pc_asked) {
            (Some(w), _) => (w, From::Card),
            (None, Some(w)) => {
                self.io.pc_asked = None;
                (w, From::Pc)
            }
            (None, None) => return Reply::Nothing,
        };
        match want {
            Want::List => {
                if ok {
                    self.take_files(body, from);
                }
                if self.io.want_card.is_none() && self.io.want_pc.is_none() && !self.io.asking() {
                    self.io.status = Text::of(if self.io.n_rows == 0 {
                        b""
                    } else {
                        b"Choose a file"
                    });
                }
            }
            Want::File(_) if !ok => {
                self.io.status = Text::of(match from {
                    From::Card => b"The card's file couldn't be read",
                    From::Pc => b"No connection to your PC (the dock)",
                });
            }
            Want::File(_) => self.import(body, cx),
            Want::Put => {
                self.io.export = None;
                self.io.status = Text::of(match (ok, from) {
                    (true, From::Card) => b"Saved in the card's RetroPane folder",
                    (true, From::Pc) => b"Saved in Documents\\RetroPane on your PC",
                    (false, From::Card) => b"The card didn't take the file",
                    (false, From::Pc) => b"No connection to your PC (the dock)",
                });
            }
            Want::Pics => {
                self.io.n_card_pics = 0;
                if ok {
                    for line in body.split(|&c| c == b'\n') {
                        let Some(bar) = line.iter().position(|&c| c == b'|') else {
                            continue;
                        };
                        let num = line[..bar].iter().try_fold(0u8, |v, &c| {
                            c.is_ascii_digit()
                                .then(|| v.checked_mul(10)?.checked_add(c - b'0'))
                                .flatten()
                        });
                        if let Some(n) = num
                            && self.io.n_card_pics < self.io.card_pics.len()
                        {
                            self.io.card_pics[self.io.n_card_pics] =
                                (n, Text::of(&line[bar + 1..]));
                            self.io.n_card_pics += 1;
                        }
                    }
                }
            }
            Want::Pic(_, at) => self.pic_piece(ok, body, at, cx),
        }
        Reply::Redraw
    }

    /// A list of "n|name" lines added to the rows.
    fn take_files(&mut self, body: &[u8], from: From) {
        for line in body.split(|&c| c == b'\n') {
            let Some(bar) = line.iter().position(|&c| c == b'|') else {
                continue;
            };
            let Some(n) = line[..bar].iter().try_fold(0u8, |v, &c| {
                c.is_ascii_digit()
                    .then(|| v.checked_mul(10)?.checked_add(c - b'0'))
                    .flatten()
            }) else {
                continue;
            };
            if self.io.n_rows < ROWS {
                self.io.rows[self.io.n_rows] = (from, n, Text::of(&line[bar + 1..]));
                self.io.n_rows += 1;
            }
        }
    }

    // ---- Import ------------------------------------------------------

    /// A vCard file read in: every person kept with their picture, then
    /// the doubles offered as one merge each.
    fn import(&mut self, body: &[u8], cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let home = rp_pim::phone::plan(*b"US").unwrap_or(&rp_pim::phone::PLANS[0]);
        let (mut people, mut pictures, mut skipped, mut full) = (0u32, 0u32, 0u32, false);
        {
            let Ok(mut guard) = cell.try_borrow_mut() else {
                return;
            };
            let Some(store) = guard.as_mut() else { return };
            let Self {
                work,
                scratch,
                dec,
                pic_buf,
                ..
            } = self;
            vcard::each(body, work, |c| {
                if c.is_empty() {
                    return true;
                }
                scratch.fill(c, home);
                let Ok(id) = store.put(Kind::Contact, None, &scratch.fields(c)) else {
                    full = true;
                    return false;
                };
                people += 1;
                skipped += u32::from(c.skipped);
                if let Some(p) = c.photo
                    && picture_of(dec, pic_buf, cx, store, id, body, p)
                {
                    pictures += 1;
                }
                true
            });
        }
        self.seen = None;
        let mut a = [0u8; 8];
        let an = digits(people as usize, &mut a);
        let mut b = [0u8; 8];
        let bn = digits(pictures as usize, &mut b);
        self.io.status = if full {
            join(&[b"Store full after ", &a[..an], b" people"])
        } else if skipped > 0 {
            let mut s = [0u8; 8];
            let sn = digits(skipped as usize, &mut s);
            join(&[b"Imported ", &a[..an], b"; ", &s[..sn], b" fields not kept"])
        } else {
            join(&[b"Imported ", &a[..an], b" people, ", &b[..bn], b" pictures"])
        };
        // The doubles, each offered as one at once.
        if let Some(g) = cx.pim.and_then(|c| c.try_borrow().ok())
            && let Some(s) = g.as_ref()
        {
            self.io.n_pairs = doubles(s, &mut self.io.pairs);
        }
        let status = self.io.status;
        if self.io.n_pairs > 0 {
            self.go(View::Merge);
        } else {
            self.go(View::Tools);
        }
        self.io.status = status;
    }

    // ---- Export ------------------------------------------------------

    fn start_export(&mut self, cx: &Ctx, to: From) {
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return;
        };
        let Some(store) = guard.as_ref() else { return };
        let mut x = Export {
            len: 0,
            rec: 0,
            rec_at: 0,
            changes: store.changes(),
            day: cx.now.day,
        };
        let v = if self.io.v4 { Version::V4 } else { Version::V3 };
        let mut len = 0;
        emit_from(
            store,
            &mut self.work,
            &mut self.jpeg_buf,
            v,
            0,
            &mut |_, b| {
                len += b.len();
                true
            },
        );
        x.len = len;
        drop(guard);
        self.io.export = Some(x);
        match to {
            From::Card => self.io.want_card = Some(Want::Put),
            From::Pc => self.io.want_pc = Some(Want::Put),
        }
        self.io.status = Text::of(b"Saving...");
    }

    /// The export's bytes from `at` into `out`.
    pub(crate) fn io_body(&mut self, at: usize, out: &mut [u8], cx: &Ctx) -> usize {
        let Some(mut x) = self.io.export else {
            return 0;
        };
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return 0;
        };
        let Some(store) = guard.as_ref() else {
            return 0;
        };
        if store.changes() != x.changes || at >= x.len {
            return 0;
        }
        if at < x.rec_at {
            x.rec = 0;
            x.rec_at = 0;
        }
        let v = if self.io.v4 { Version::V4 } else { Version::V3 };
        let room = out.len();
        let (mut n, mut pos, mut first) = (0, x.rec_at, (x.rec, x.rec_at));
        emit_from(
            store,
            &mut self.work,
            &mut self.jpeg_buf,
            v,
            x.rec,
            &mut |k, b| {
                if pos < at && k > first.0 {
                    first = (k, pos);
                }
                for &c in b {
                    if pos >= at && pos - at < room {
                        out[pos - at] = c;
                        n = n.max(pos - at + 1);
                    }
                    pos += 1;
                }
                pos < at + room
            },
        );
        x.rec = first.0;
        x.rec_at = first.1;
        self.io.export = Some(x);
        n
    }

    // ---- Duplicates ----------------------------------------------------

    /// The pair shown, if any is left.
    fn pair(&self) -> Option<(u32, u32)> {
        self.io.pairs[..self.io.n_pairs].first().copied()
    }

    /// After a pair is answered: the doubles counted again (a merge may
    /// have made or ended others), back to the tools when none is left.
    fn next_pair(&mut self, cx: &Ctx) {
        if let Some(g) = cx.pim.and_then(|c| c.try_borrow().ok())
            && let Some(s) = g.as_ref()
        {
            self.io.n_pairs = doubles(s, &mut self.io.pairs);
        }
        if self.io.n_pairs == 0 {
            let status = self.io.status;
            self.go(View::Tools);
            self.io.status = status;
        }
    }

    /// The pair shown made one: the lower id kept with every field of
    /// both, the picture the kept one lacks taken from the other, the
    /// other removed.
    fn merge_pair(&mut self, cx: &Ctx) {
        let Some((a, b)) = self.pair() else { return };
        let home = rp_pim::phone::plan(*b"US").unwrap_or(&rp_pim::phone::PLANS[0]);
        let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok()) else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let (Some(ra), Some(rb)) = (store.get(a), store.get(b)) else {
            return;
        };
        let fav = ra.num(tag::FAVOURITE) == Some(1) || rb.num(tag::FAVOURITE) == Some(1);
        vcard::read_into(&ra, &mut self.work);
        vcard::read_into(&rb, &mut self.other);
        vcard::merge(&mut self.work, &self.other);
        self.scratch.fill(&self.work, home);
        let f = self.scratch.fields(&self.work);
        let one = 1u32.to_le_bytes();
        let mut all = [(0u8, &[][..]); vcard::FIELDS + 1];
        all[..vcard::FIELDS].copy_from_slice(&f);
        all[vcard::FIELDS] = (tag::FAVOURITE, if fav { &one } else { &[] });
        if store.change(a, &all).is_err() {
            self.io.status = Text::of(b"The store is full");
            return;
        }
        // The picture: the kept one's, else the other's moved over.
        let (pa, na) = Self::photo_pages(store, a);
        let (pb, nb) = Self::photo_pages(store, b);
        let _ = pa;
        for &page in &pb[..nb] {
            if na == 0 {
                let _ = store.change(page, &[(tag::PARENT, &a.to_le_bytes())]);
            } else {
                let _ = store.remove(page);
            }
        }
        let _ = store.remove(b);
        drop(guard);
        self.seen = None;
        self.io.status = Text::of(b"Merged into one");
    }

    /// The pair shown kept as two, and remembered so it isn't asked again.
    fn keep_apart(&mut self, cx: &Ctx) {
        let Some(pair) = self.pair() else { return };
        let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok()) else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let (mut list, mut n, id) = apart(store);
        if n == APART {
            // The oldest answer gives way.
            list.copy_within(1.., 0);
            n -= 1;
        }
        list[n] = pair;
        n += 1;
        let mut bytes = [0u8; APART * 8];
        for (k, (x, y)) in list[..n].iter().enumerate() {
            bytes[k * 8..k * 8 + 4].copy_from_slice(&x.to_le_bytes());
            bytes[k * 8 + 4..k * 8 + 8].copy_from_slice(&y.to_le_bytes());
        }
        let text = &bytes[..n * 8];
        let _ = match id {
            Some(id) => store.change(id, &[(tag::TEXT, text)]).map(|_| ()),
            None => store
                .put(
                    Kind::Data,
                    None,
                    &[(tag::NAME, b"apart"), (tag::TEXT, text)],
                )
                .map(|_| ()),
        };
        self.io.status = Text::of(b"Kept as two people");
    }

    pub(crate) fn merge_view(&mut self, f: &mut Frame, cx: &Ctx) {
        let Some((a, b)) = self.pair() else { return };
        let mut k = [0u8; 8];
        let kn = digits(self.io.n_pairs, &mut k);
        let sub: Text<48> = join(&[
            &k[..kn],
            if self.io.n_pairs == 1 {
                b" pair to look at"
            } else {
                b" pairs to look at"
            },
        ]);
        let y = kit::large_title(f, cx, 36, b"The same person?", sub.get(), tint::CONTACTS) + 8;
        let shared = {
            let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
                return;
            };
            let Some(store) = guard.as_ref() else { return };
            let (Some(ra), Some(rb)) = (store.get(a), store.get(b)) else {
                return;
            };
            vcard::read_into(&ra, &mut self.work);
            vcard::read_into(&rb, &mut self.other);
            let (ca, cb) = (&self.work, &self.other);
            // What they share, as the card shows it.
            let mut shared = Text::<48>::EMPTY;
            for p in &ca.phones[..ca.n_phones] {
                let key = rp_pim::contact::phone_key(p.value.get());
                if key.is_some()
                    && cb.phones[..cb.n_phones]
                        .iter()
                        .any(|q| rp_pim::contact::phone_key(q.value.get()) == key)
                {
                    shared = join(&[b"Both have ", super::shown(p.value.get()).get()]);
                    break;
                }
            }
            if shared.is_empty() {
                for e in &ca.emails[..ca.n_emails] {
                    if cb.emails[..cb.n_emails]
                        .iter()
                        .any(|q| rp_pim::contact::same_email(q.value.get(), e.value.get()))
                    {
                        shared = join(&[b"Both have ", e.value.get()]);
                        break;
                    }
                }
            }
            shared
        };
        // The two side by side.
        let colw = (WIDE - 8) / 2;
        for k in 0..2 {
            let c = if k == 0 { &self.work } else { &self.other };
            let r = Rect::new(MARGIN + k as u16 * (colw + 8), y, colw, 236);
            kit::panel(f, cx, r);
            let mut nm = [0u8; 64];
            let mut nn = 0;
            for &ch in c
                .first
                .get()
                .iter()
                .chain(if c.first.is_empty() { &b""[..] } else { b" " })
                .chain(c.last.get())
            {
                if nn < nm.len() {
                    nm[nn] = ch;
                    nn += 1;
                }
            }
            let disc = self.big_for(&nm[..nn]);
            let id = if k == 0 { a } else { b };
            self.avatar(
                f,
                cx,
                Some(id),
                disc,
                (r.x + (colw - 64) / 2, r.y + 12),
                64,
                cx.fonts.title,
                &nm[..nn],
            );
            let c = if k == 0 { &self.work } else { &self.other };
            let line = Rect::new(r.x, 0, r.w, 1);
            kit::centred(f, line, r.y + 104, cx.fonts.ui, TEXT, &nm[..nn]);
            kit::centred(f, line, r.y + 126, cx.fonts.label, DIM, c.company.get());
            let mut ly = r.y + 152;
            for p in c.phones[..c.n_phones].iter().take(2) {
                kit::centred(
                    f,
                    line,
                    ly,
                    cx.fonts.label,
                    TEXT,
                    super::shown(p.value.get()).get(),
                );
                ly += 20;
            }
            for e in c.emails[..c.n_emails].iter().take(2) {
                kit::centred(f, line, ly, cx.fonts.label, TEXT, e.value.get());
                ly += 20;
            }
        }
        // What they share, and what the one card will hold.
        let r = Rect::new(MARGIN, y + 246, WIDE, 76);
        kit::tinted(f, cx, r, tint::CONTACTS, 0x30);
        kit::text(f, r.x + 14, r.y + 26, cx.fonts.ui, TEXT, shared.get());
        // What the one card will hold, counted (no third card made).
        let (ca, cb) = (&self.work, &self.other);
        let new_phones = cb.phones[..cb.n_phones]
            .iter()
            .filter(|p| {
                let k = rp_pim::contact::phone_key(p.value.get());
                !ca.phones[..ca.n_phones].iter().any(|q| {
                    q.value.get() == p.value.get()
                        || (k.is_some() && rp_pim::contact::phone_key(q.value.get()) == k)
                })
            })
            .count();
        let new_emails = cb.emails[..cb.n_emails]
            .iter()
            .filter(|e| {
                !ca.emails[..ca.n_emails]
                    .iter()
                    .any(|q| rp_pim::contact::same_email(q.value.get(), e.value.get()))
            })
            .count();
        struct Merged {
            n_phones: usize,
            n_emails: usize,
            address: bool,
            birthday: bool,
        }
        let merged = Merged {
            n_phones: (ca.n_phones + new_phones).min(vcard::PHONES),
            n_emails: (ca.n_emails + new_emails).min(vcard::EMAILS),
            address: !ca.address().is_empty() || !cb.address().is_empty(),
            birthday: ca.birthday.or(cb.birthday).is_some(),
        };
        let mut pn = [0u8; 8];
        let pl = digits(merged.n_phones, &mut pn);
        let mut en = [0u8; 8];
        let el = digits(merged.n_emails, &mut en);
        let mut parts: Text<64> = join(&[
            b"As one: ",
            &pn[..pl],
            if merged.n_phones == 1 {
                b" number, "
            } else {
                b" numbers, "
            },
            &en[..el],
            if merged.n_emails == 1 {
                b" e-mail"
            } else {
                b" e-mails"
            },
        ]);
        if merged.address {
            parts = join(&[parts.get(), b", address"]);
        }
        if merged.birthday {
            parts = join(&[parts.get(), b", birthday"]);
        }
        kit::fitted(
            f,
            r.x + 14,
            r.y + 54,
            cx.fonts.ui,
            DIM,
            parts.get(),
            r.w - 28,
        );
        kit::key(f, cx, MERGE_KEY, kit::KeyState::On);
        kit::centred(
            f,
            MERGE_KEY,
            MERGE_KEY.y + 33,
            cx.fonts.ui,
            TEXT,
            b"Merge into one",
        );
        kit::key(f, cx, APART_KEY, kit::KeyState::Rest);
        kit::centred(
            f,
            APART_KEY,
            APART_KEY.y + 33,
            cx.fonts.ui,
            TEXT,
            b"Keep both",
        );
    }

    // ---- Pictures from the card -----------------------------------------

    /// The chooser asks the card for its pictures too.
    pub(crate) fn ask_card_pics(&mut self) {
        self.io.n_card_pics = 0;
        self.io.want_card = Some(Want::Pics);
    }

    /// Card picture `k` (of the chooser's) chosen for person `id`.
    pub(crate) fn pick_card_pic(&mut self, cx: &Ctx, id: u32, k: usize) {
        let Some(&(n, _)) = self.io.card_pics[..self.io.n_card_pics].get(k) else {
            return;
        };
        let Some(pics) = cx.pics else { return };
        if let Some(old) = self.io.job.take() {
            pics.free(old.pic);
        }
        let Some(pic) = pics.alloc(jpeg::SIDE as u16, jpeg::SIDE as u16) else {
            self.photo_note = b"No room to read the picture";
            return;
        };
        pics.write(pic, |px| px.fill(0));
        self.dec = Decoder::new();
        self.io.job = Some(PicJob { id, n, pic });
        self.io.want_card = Some(Want::Pic(n, 0));
        self.photo_note = b"Reading the picture...";
    }

    /// A piece of the card's picture: decoded on, the next piece asked,
    /// or the picture kept.
    fn pic_piece(&mut self, ok: bool, body: &[u8], at: u64, cx: &Ctx) {
        let (Some(job), Some(pics)) = (self.io.job, cx.pics) else {
            return;
        };
        let fail = |this: &mut Self, why: &'static [u8]| {
            pics.free(job.pic);
            this.io.job = None;
            this.photo_note = why;
        };
        let (Some(size), true) = (body.get(..4), ok) else {
            return fail(self, b"The card's picture couldn't be read");
        };
        let size = u64::from(u32::from_le_bytes([size[0], size[1], size[2], size[3]]));
        let data = &body[4..];
        let last = at + data.len() as u64 >= size;
        let mut got = Ok(Progress::Done);
        let dec = &mut self.dec;
        pics.write(job.pic, |px| got = dec.feed(data, last, px));
        match got {
            Ok(Progress::More(n)) if n > 0 || !data.is_empty() => {
                self.io.want_card = Some(Want::Pic(job.n, at + n as u64));
            }
            Ok(Progress::More(_)) => fail(self, b"The picture ends too soon"),
            Ok(Progress::Done) => {
                let kept = self.keep_decoded(cx, job.id, job.pic);
                pics.free(job.pic);
                self.io.job = None;
                if kept {
                    self.seen = None;
                    if let View::Photo(i) = self.view {
                        self.go(View::Card(i));
                    }
                } else {
                    self.photo_note = b"No room left to keep it";
                }
            }
            Err(_) => fail(self, b"Not a picture this can read (JPEG only)"),
        }
    }

    /// The decoded square in `pic` kept as person `id`'s picture.
    fn keep_decoded(&mut self, cx: &Ctx, id: u32, pic: Pic) -> bool {
        let Some(pics) = cx.pics else { return false };
        Self::photo_drop(cx, id);
        let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok()) else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let dec = &self.dec;
        let mut kept = false;
        pics.write(pic, |px| {
            let get = |x: usize, y: usize| {
                let (r, g, b) = dec.rgb(px, x, y);
                (i32::from(r), i32::from(g), i32::from(b))
            };
            kept = photo::encode_with(&get, |page| {
                store
                    .put(
                        Kind::Ink,
                        None,
                        &[(tag::PARENT, &id.to_le_bytes()), (tag::INK, page)],
                    )
                    .is_ok()
            });
        });
        drop(guard);
        if !kept {
            Self::photo_drop(cx, id);
        }
        kept
    }
}

/// A vCard's picture decoded into person `id`'s (in a picture slot, a
/// piece of its base64 at a time): whether it was kept.
fn picture_of(
    dec: &mut Decoder,
    buf: &mut [u8; 2048],
    cx: &Ctx,
    store: &mut Store<'_>,
    id: u32,
    text: &[u8],
    p: vcard::Photo,
) -> bool {
    let Some(pics) = cx.pics else { return false };
    let Some(mut bytes) = vcard::PhotoBytes::of(text, p) else {
        return false;
    };
    let Some(pic) = pics.alloc(jpeg::SIDE as u16, jpeg::SIDE as u16) else {
        return false;
    };
    *dec = Decoder::new();
    let mut done = false;
    pics.write(pic, |px| {
        px.fill(0);
        let mut have = 0;
        let mut end = false;
        loop {
            while have < buf.len() && !end {
                match bytes.next() {
                    Some(b) => {
                        buf[have] = b;
                        have += 1;
                    }
                    None => end = true,
                }
            }
            match dec.feed(&buf[..have], end, px) {
                Ok(Progress::Done) => {
                    done = true;
                    return;
                }
                Ok(Progress::More(n)) if n > 0 => {
                    buf.copy_within(n..have, 0);
                    have -= n;
                }
                // Nothing used of a full window (headers past it) or an
                // error: no picture.
                _ => return,
            }
        }
    });
    let mut kept = false;
    if done {
        let dec = &*dec;
        pics.write(pic, |px| {
            let get = |x: usize, y: usize| {
                let (r, g, b) = dec.rgb(px, x, y);
                (i32::from(r), i32::from(g), i32::from(b))
            };
            kept = photo::encode_with(&get, |page| {
                store
                    .put(
                        Kind::Ink,
                        None,
                        &[(tag::PARENT, &id.to_le_bytes()), (tag::INK, page)],
                    )
                    .is_ok()
            });
        });
        if !kept {
            let (ids, n) = Contacts::photo_pages(store, id);
            for &page in &ids[..n.min(PHOTO_PAGES)] {
                let _ = store.remove(page);
            }
        }
    }
    pics.free(pic);
    kept
}

/// The export from record `from` on (`from` 0: the first), each piece to
/// `sink` with its record's number, until `sink` answers false. A vCard
/// file has no head or tail: its people one after another.
fn emit_from(
    store: &Store<'_>,
    work: &mut Card,
    jpeg: &mut [u8; JPEG_ROOM],
    v: Version,
    from: usize,
    sink: &mut dyn FnMut(usize, &[u8]) -> bool,
) {
    let mut go = true;
    for (k, r) in store.all(Kind::Contact).enumerate().skip(from) {
        if !go {
            return;
        }
        vcard::read_into(&r, work);
        let n = picture_jpeg(store, r.id, jpeg);
        let photo = n.map(|n| (&jpeg[..n], &b"jpeg"[..]));
        vcard::write(work, photo, v, |b| go = go && sink(k, b));
    }
}

/// The room a person's picture is written as a JPEG in.
pub(crate) const JPEG_ROOM: usize = 8192;

/// Person `id`'s kept picture as a JPEG in `out` (quality 85, lower if it
/// doesn't fit): its length; `None` without a picture.
fn picture_jpeg(store: &Store<'_>, id: u32, out: &mut [u8; JPEG_ROOM]) -> Option<usize> {
    let (ids, n) = Contacts::photo_pages(store, id);
    if n == 0 {
        return None;
    }
    let mut pages: [&[u8]; PHOTO_PAGES] = [&[]; PHOTO_PAGES];
    for (p, &page) in pages.iter_mut().zip(&ids[..n]) {
        *p = store.get(page)?.text(tag::INK);
    }
    let kept = photo::Kept::of(&pages[..n])?;
    [85, 70, 50]
        .into_iter()
        .find_map(|q| rp_pim::jpeg_out::encode(&|x, y| kept.rgb(x, y), q, out))
}

#[cfg(test)]
impl Contacts {
    /// The tools' key `k` as last drawn.
    pub(crate) fn io_tool_for_test(&self, k: usize) -> Rect {
        self.io.tools_at[k].0
    }
    /// The files' row `k` as last drawn.
    pub(crate) fn io_row_for_test(&self, k: usize) -> Option<Rect> {
        self.io.rows_at[..self.io.n_at].get(k).map(|r| r.0)
    }
    /// The merge screen's Merge key (else Keep both).
    pub(crate) const fn io_merge_key_for_test(&self, merge: bool) -> Rect {
        if merge { MERGE_KEY } else { APART_KEY }
    }
}
