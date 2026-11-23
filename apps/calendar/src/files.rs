//! Calendar's files and zones (calendar-rule0.md ".ics import and export"
//! and "Time zones"): a calendar file read from the card or the PC into a
//! calendar of its own, every appointment written out as one .ics, and the
//! device's time zone, which every view shows times in.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 read no .ics at all
//! and kept one device-wide zone set by hand; Google imports a file into a
//! calendar the user picks and keeps doubles when the same file comes
//! twice. Ours lists the card's and the PC's calendar files together (the
//! card needs no dock), reads the chosen one into a new calendar named
//! after the file (its own colour: shown, hidden or deleted as one),
//! leaves out an appointment already kept (same title, start and zone) and
//! says how many, and writes the whole calendar a piece at a time (never
//! held whole: 2,048 appointments would be 600 KB) with a VTIMEZONE for
//! each zone it uses. The device's zone is the one the owner picked, else
//! guessed from the dock PC's offset, a US zone first.

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Event, Frame, Key, MARGIN, Rect, Reply, WIDE};
use rp_pim::date;
use rp_pim::net::Short;
use rp_pim::store::{Kind, tag};
use rp_pim::zone::{self, Zone};

use super::{Asked, Buf, Calendar, Text, View, clock};

/// Where a file comes from or goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum From {
    /// The device's own card.
    Card,
    /// The PC on the dock.
    Pc,
}

/// What the files' screen is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// A file chosen to read.
    Import,
    /// Where to write the calendar.
    Export,
}

/// A question about files, to the card or the PC.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Want {
    /// The calendar files there.
    List,
    /// File `n` of the list.
    File(u8),
    /// The calendar written there.
    Put,
}

/// The calendar being written: its length, the zones it uses, how far it
/// has been sent (the record and the byte it starts at), and the store's
/// change count it was counted at (a change in between stops it).
#[derive(Clone, Copy, Debug)]
pub struct Export {
    len: usize,
    zones: [Short<40>; 8],
    n_zones: usize,
    rec: usize,
    rec_at: usize,
    changes: u32,
    /// The day its name carries (today).
    day: i32,
}

/// The rows a files' screen lists.
const ROWS: usize = 10;
/// The zone picker's rows.
const ZONE_ROWS: usize = 9;
/// The time zone's row on the calendars' screen.
pub const ZONE_ROW: Rect = Rect::new(MARGIN, 436, WIDE, 40);
/// The import and export keys there.
pub const IMPORT_KEY: Rect = Rect::new(MARGIN, 480, WIDE / 2 - 4, 40);
pub const EXPORT_KEY: Rect = Rect::new(MARGIN + WIDE / 2 + 4, 480, WIDE / 2 - 4, 40);
/// The pickers' search field.
const FIELD: Rect = Rect::new(MARGIN, 36 + kit::TITLE_BAND + 4, WIDE, 38);
/// The export's two keys.
const TO_CARD: Rect = Rect::new(MARGIN, 220, WIDE, 52);
const TO_PC: Rect = Rect::new(MARGIN, 284, WIDE, 52);

/// The state of the files' and the zones' screens.
#[derive(Clone, Copy, Debug)]
pub struct Io {
    pub mode: Mode,
    rows: [(From, u8, Text); ROWS],
    n_rows: usize,
    rows_at: [(Rect, u8); ROWS],
    n_at: usize,
    want_card: Option<Want>,
    want_pc: Option<Want>,
    card_asked: Option<Want>,
    pc_asked: Option<Want>,
    /// The file chosen (its name, a new calendar's).
    chosen: Text,
    export: Option<Export>,
    pub status: &'static [u8],
    /// The zone picker's search, its rows on screen, the first shown.
    query: [u8; 24],
    query_n: usize,
    pub typing: bool,
    zones_at: [(Rect, Option<Zone>); ZONE_ROWS],
    n_zones: usize,
    zone_first: usize,
    /// The device's zone (chosen, else guessed) as the views use it.
    pub device: Option<Zone>,
    pub guessed: bool,
}

impl Io {
    /// Nothing under way.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mode: Mode::Import,
            rows: [(From::Card, 0, Text::EMPTY); ROWS],
            n_rows: 0,
            rows_at: [(Rect::new(0, 0, 0, 0), 0); ROWS],
            n_at: 0,
            want_card: None,
            want_pc: None,
            card_asked: None,
            pc_asked: None,
            chosen: Text::EMPTY,
            export: None,
            status: b"",
            query: [0; 24],
            query_n: 0,
            typing: false,
            zones_at: [(Rect::new(0, 0, 0, 0), None); ZONE_ROWS],
            n_zones: 0,
            zone_first: 0,
            device: None,
            guessed: false,
        }
    }

    /// The rows a stylus or the D-pad can choose on the screen shown.
    pub fn targets(&self, view: View) -> impl Iterator<Item = Rect> + '_ {
        let files = view == View::Files;
        let zones = view == View::Zones;
        let export = files && self.mode == Mode::Export;
        self.rows_at[..self.n_at]
            .iter()
            .map(|r| r.0)
            .filter(move |_| files && !export)
            .chain([TO_CARD, TO_PC].into_iter().filter(move |_| export))
            .chain(core::iter::once(FIELD).filter(move |_| zones))
            .chain(
                self.zones_at[..self.n_zones]
                    .iter()
                    .map(|r| r.0)
                    .filter(move |_| zones),
            )
    }
}

impl Default for Io {
    fn default() -> Self {
        Self::new()
    }
}

/// The tzdata name of a kept zone record (`Kind::Data` named "zone").
fn chosen_zone(store: &rp_pim::store::Store<'_>) -> Option<Zone> {
    store
        .all(Kind::Data)
        .find(|r| r.text(tag::NAME) == b"zone")
        .and_then(|r| zone::find(r.text(tag::TEXT)))
}

/// The device's zone: the one chosen, else guessed from the dock PC's
/// offset now (a US zone first). Whether it was guessed.
#[must_use]
pub fn device_zone(store: &rp_pim::store::Store<'_>, cx: &Ctx) -> (Option<Zone>, bool) {
    if let Some(z) = chosen_zone(store) {
        return (Some(z), false);
    }
    let local = i64::from(cx.now.day) * 1440 + i64::from(cx.now.minute);
    (
        zone::guess(local - i64::from(cx.utc_offset), cx.utc_offset),
        true,
    )
}

/// A zone's name as people say it with its offset now: "New York (UTC-5)".
fn zone_line(z: Zone, utc: i64) -> Buf<48> {
    let mut place = [0u8; 32];
    let n = z.place(&mut place);
    let off = z.rule().offset_at(utc);
    let mut b = Buf::new();
    b.s(&place[..n]).s(b" (UTC");
    if off != 0 {
        b.s(if off < 0 { b"-" } else { b"+" })
            .num(off.unsigned_abs() / 60);
        if !off.unsigned_abs().is_multiple_of(60) {
            b.s(b":").num(off.unsigned_abs() % 60);
        }
    }
    b.s(b")");
    b
}

/// `wall` in zone `name` as the device's wall time (unchanged with no
/// device zone, a floating time, or a zone no table has).
#[must_use]
pub fn shown(device: Option<Zone>, name: &[u8], wall: u32) -> u32 {
    let (Some(d), false) = (device, name.is_empty()) else {
        return wall;
    };
    let Some(z) = zone::find(name) else {
        return wall;
    };
    if z == d {
        return wall;
    }
    u32::try_from(z.rule().to(i64::from(wall), &d.rule())).unwrap_or(wall)
}

/// The device's wall time `wall` as the wall time of zone `name` (the
/// editor's time kept in the appointment's own zone).
#[must_use]
pub fn kept(device: Option<Zone>, name: &[u8], wall: u32) -> u32 {
    let (Some(d), false) = (device, name.is_empty()) else {
        return wall;
    };
    let Some(z) = zone::find(name) else {
        return wall;
    };
    if z == d {
        return wall;
    }
    u32::try_from(d.rule().to(i64::from(wall), &z.rule())).unwrap_or(wall)
}

fn digits(n: usize) -> Buf<8> {
    let mut b = Buf::new();
    b.num(n as u32);
    b
}

impl Calendar {
    // ---- The calendars' screen's new rows ----------------------------

    /// The time zone's row and the import and export keys (under the
    /// calendars' list, over its Get key).
    pub(crate) fn files_rows(&mut self, f: &mut Frame, cx: &Ctx) {
        kit::panel(f, cx, ZONE_ROW);
        kit::text(
            f,
            ZONE_ROW.x + 14,
            ZONE_ROW.y + 26,
            cx.fonts.label,
            DIM,
            b"TIME ZONE",
        );
        let utc =
            i64::from(cx.now.day) * 1440 + i64::from(cx.now.minute) - i64::from(cx.utc_offset);
        match self.io.device {
            Some(z) => {
                let l = zone_line(z, utc);
                kit::fitted(
                    f,
                    ZONE_ROW.x + 120,
                    ZONE_ROW.y + 26,
                    cx.fonts.ui,
                    TEXT,
                    l.get(),
                    260,
                );
            }
            None => kit::text(
                f,
                ZONE_ROW.x + 120,
                ZONE_ROW.y + 26,
                cx.fonts.ui,
                DIM,
                b"Not known yet",
            ),
        }
        kit::right(
            f,
            ZONE_ROW.right() - 14,
            ZONE_ROW.y + 26,
            cx.fonts.label,
            if self.io.guessed { DIM } else { tint::CALENDAR },
            if self.io.guessed {
                b"FROM THE PC"
            } else {
                b"CHOSEN"
            },
        );
        for (r, word) in [
            (IMPORT_KEY, &b"Import .ics"[..]),
            (EXPORT_KEY, b"Export .ics"),
        ] {
            kit::key(f, cx, r, kit::KeyState::Rest);
            kit::centred(f, r, r.y + 27, cx.fonts.ui, TEXT, word);
        }
    }

    /// A tap on the calendars' screen's new rows: whether it was one.
    pub(crate) fn files_tap(&mut self, x: u16, y: u16) -> bool {
        if ZONE_ROW.contains_point(x, y) {
            self.view = View::Zones;
            self.io.typing = false;
            self.io.query_n = 0;
            self.io.zone_first = 0;
            return true;
        }
        if IMPORT_KEY.contains_point(x, y) {
            self.io.mode = Mode::Import;
            self.io.n_rows = 0;
            self.io.want_card = Some(Want::List);
            self.io.want_pc = Some(Want::List);
            self.io.status = b"Looking on the card and your PC...";
            self.view = View::Files;
            return true;
        }
        if EXPORT_KEY.contains_point(x, y) {
            self.io.mode = Mode::Export;
            self.io.status = b"";
            self.view = View::Files;
            return true;
        }
        false
    }

    // ---- The files' screen -------------------------------------------

    pub(crate) fn files_view(&mut self, f: &mut Frame, cx: &Ctx) {
        self.io.n_at = 0;
        let import = self.io.mode == Mode::Import;
        kit::large_title(
            f,
            cx,
            36,
            if import {
                b"Import a calendar"
            } else {
                b"Export the calendar"
            },
            if import {
                b".ics files on the card and your PC"
            } else {
                b"Every appointment, one .ics file"
            },
            tint::CALENDAR,
        );
        let mut y = FIELD.y;
        if import {
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
                    tint::CALENDAR,
                    match from {
                        From::Card => b"CARD",
                        From::Pc => b"PC",
                    },
                );
                self.io.rows_at[self.io.n_at] = (r, k as u8);
                self.io.n_at += 1;
                y += 50;
            }
            if self.io.n_rows == 0 && self.io.want_card.is_none() && self.io.card_asked.is_none() {
                kit::text(
                    f,
                    MARGIN + 8,
                    y + 26,
                    cx.fonts.ui,
                    DIM,
                    b"No .ics file on the card or the PC yet",
                );
                kit::text(
                    f,
                    MARGIN + 8,
                    y + 50,
                    cx.fonts.ui,
                    DIM,
                    b"Put one in the card's RetroPane folder,",
                );
                kit::text(
                    f,
                    MARGIN + 8,
                    y + 74,
                    cx.fonts.ui,
                    DIM,
                    b"or Documents\\RetroPane on your PC",
                );
            }
        } else {
            let mut name = Buf::<40>::new();
            export_name(cx.now.day, &mut name);
            kit::text(
                f,
                MARGIN + 8,
                FIELD.y + 26,
                cx.fonts.label,
                DIM,
                b"THE FILE",
            );
            kit::fitted(
                f,
                MARGIN + 100,
                FIELD.y + 27,
                cx.fonts.ui,
                TEXT,
                name.get(),
                WIDE - 110,
            );
            for (r, word, sub) in [
                (
                    TO_CARD,
                    &b"Save to the card"[..],
                    &b"In its RetroPane folder"[..],
                ),
                (
                    TO_PC,
                    b"Save to your PC",
                    b"Documents\\RetroPane, over the dock",
                ),
            ] {
                kit::key(f, cx, r, kit::KeyState::On);
                kit::text(f, r.x + 16, r.y + 24, cx.fonts.ui, TEXT, word);
                kit::text(f, r.x + 16, r.y + 44, cx.fonts.label, DIM, sub);
            }
            y = TO_PC.bottom() + 8;
        }
        if !self.io.status.is_empty() {
            kit::text(
                f,
                MARGIN + 8,
                y + 26,
                cx.fonts.ui,
                kit::FOCUS,
                self.io.status,
            );
        }
    }

    /// An event on the files' or the zones' screen.
    pub(crate) fn files_event(&mut self, e: Event, cx: &Ctx) -> Reply {
        if self.view == View::Zones {
            return self.zones_event(e, cx);
        }
        match e {
            Event::Key(Key::SoftLeft) => {
                self.view = View::Calendars;
                Reply::Redraw
            }
            Event::PenUp { x, y } if self.io.mode == Mode::Export => {
                let to = if TO_CARD.contains_point(x, y) {
                    From::Card
                } else if TO_PC.contains_point(x, y) {
                    From::Pc
                } else {
                    return Reply::Nothing;
                };
                self.start_export(cx, to);
                Reply::Redraw
            }
            Event::PenUp { x, y } => {
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
                self.io.status = b"Reading the file...";
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    // ---- The questions ---------------------------------------------

    /// The card's question, if one waits.
    pub(crate) fn files_card_ask(&mut self) -> Option<rp_app::NetAsk> {
        let want = self.io.want_card.take()?;
        self.io.card_asked = Some(want);
        Some(self.files_ask(want, true))
    }

    /// The PC's question, if one waits.
    pub(crate) fn files_net_ask(&mut self) -> Option<rp_app::NetAsk> {
        let want = self.io.want_pc.take()?;
        self.io.pc_asked = Some(want);
        self.asked = Asked::Nothing;
        Some(self.files_ask(want, false))
    }

    fn files_ask(&self, want: Want, card: bool) -> rp_app::NetAsk {
        let mut arg = Buf::<64>::new();
        let what: &'static [u8] = match (want, card) {
            (Want::List, true) => {
                arg.s(b"ics");
                b"CARDFILES"
            }
            (Want::List, false) => {
                arg.s(b"ics");
                b"FILES"
            }
            (Want::File(n), c) => {
                arg.num(u32::from(n)).s(b" ics");
                if c { b"CARDFILE" } else { b"FILE" }
            }
            (Want::Put, c) => {
                let len = self.io.export.map_or(0, |x| x.len);
                arg.num(len as u32).s(b" ");
                let mut name = Buf::<40>::new();
                export_name(self.io.export.map_or(0, |x| x.day), &mut name);
                arg.s(name.get());
                if c { b"CARDPUT" } else { b"PUT" }
            }
        };
        rp_app::NetAsk {
            what,
            arg: rp_app::Text::of(arg.get()),
        }
    }

    /// Whether an answer is the files' (the card's, or the PC's).
    pub(crate) const fn files_asking(&self) -> bool {
        self.io.card_asked.is_some() || self.io.pc_asked.is_some()
    }

    /// An answer to a files' question.
    pub(crate) fn files_answer(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
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
                if self.io.want_card.is_none()
                    && self.io.card_asked.is_none()
                    && self.io.want_pc.is_none()
                    && self.io.pc_asked.is_none()
                {
                    self.io.status = if self.io.n_rows == 0 {
                        b""
                    } else {
                        b"Choose a file"
                    };
                }
            }
            Want::File(_) if !ok => {
                self.io.status = match from {
                    From::Card => b"The card's file couldn't be read",
                    From::Pc => b"No connection to your PC (the dock)",
                };
            }
            Want::File(_) => {
                self.import(body, cx);
                self.view = View::Calendars;
            }
            Want::Put => {
                self.io.export = None;
                self.status = match (ok, from) {
                    (true, From::Card) => b"Saved in the card's RetroPane folder",
                    (true, From::Pc) => b"Saved in Documents\\RetroPane on your PC",
                    (false, From::Card) => b"The card didn't take the file",
                    (false, From::Pc) => b"No connection to your PC (the dock)",
                };
                self.view = View::Calendars;
            }
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

    // ---- Import --------------------------------------------------------

    /// File `body` read into a new calendar named after the file: every
    /// event not already kept (the same title, start and zone), and how
    /// many in the status.
    fn import(&mut self, body: &[u8], cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let name = self.io.chosen;
        let stem = {
            let n = name.get();
            &n[..n.iter().rposition(|&c| c == b'.').unwrap_or(n.len())]
        };
        let used = store
            .all(Kind::Calendar)
            .fold(0u8, |m, r| m | 1 << (r.num(tag::COLOUR).unwrap_or(0) % 8));
        let colour = (0..8u32)
            .find(|&k| used & (1 << k) == 0)
            .unwrap_or(3)
            .to_le_bytes();
        let Ok(cal) = store.put(
            Kind::Calendar,
            None,
            &[(tag::NAME, stem), (tag::COLOUR, &colour)],
        ) else {
            self.status = b"The calendar store is full";
            return;
        };
        let (mut put, mut had, mut full) = (0u32, 0u32, false);
        rp_pim::ics::ics_each(body, |e| {
            let kept = store.all(Kind::Appointment).any(|r| {
                r.num(tag::START) == Some(e.start)
                    && r.text(tag::TITLE) == e.title.get()
                    && r.text(tag::ZONE) == e.zone.get()
            });
            if kept {
                had += 1;
                return true;
            }
            let fields = super::EventFields::of(e, cal);
            if store.put(Kind::Appointment, None, &fields.list()).is_ok() {
                put += 1;
                true
            } else {
                full = true;
                false
            }
        });
        if put == 0 {
            // Nothing new: no empty calendar left behind.
            let _ = store.remove(cal);
        }
        self.seen = None;
        let mut b = Buf::<64>::new();
        b.s(if full { b"Store full: " } else { b"Imported " })
            .num(put);
        if had > 0 {
            b.s(b" (").num(had).s(b" already here)");
        } else {
            b.s(if put == 1 {
                b" appointment"
            } else {
                b" appointments"
            });
        }
        self.io.status = b"";
        self.set_status(b.get());
    }

    /// The status line made from text that isn't `'static` (a count):
    /// kept in the app's own room.
    fn set_status(&mut self, s: &[u8]) {
        self.status_text = Text::of(s);
        self.status = b"\x01";
    }

    // ---- Export --------------------------------------------------------

    /// The calendar counted (its length and the zones it uses) and asked
    /// to be written to the card or the PC.
    fn start_export(&mut self, cx: &Ctx, to: From) {
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return;
        };
        let Some(store) = guard.as_ref() else { return };
        let mut x = Export {
            len: 0,
            zones: [Short::new(); 8],
            n_zones: 0,
            rec: 0,
            rec_at: 0,
            changes: store.changes(),
            day: cx.now.day,
        };
        let device = self.io.device;
        for r in store.all(Kind::Appointment) {
            let z = zone_of(&r, device);
            if !z.is_empty()
                && !x.zones[..x.n_zones].iter().any(|k| k.get() == z.get())
                && x.n_zones < 8
            {
                x.zones[x.n_zones] = z;
                x.n_zones += 1;
            }
        }
        let mut len = 0;
        emit(store, &x, device, cx, &mut |b| len += b.len());
        x.len = len;
        drop(guard);
        self.io.export = Some(x);
        match to {
            From::Card => self.io.want_card = Some(Want::Put),
            From::Pc => self.io.want_pc = Some(Want::Put),
        }
        self.io.status = b"Saving...";
    }

    /// The export's bytes from `at` into `out`.
    pub(crate) fn files_body(&mut self, at: usize, out: &mut [u8], cx: &Ctx) -> usize {
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
        // From the record the last piece ended in (pieces come in order).
        if at < x.rec_at {
            x.rec = 0;
            x.rec_at = 0;
        }
        let device = self.io.device;
        let room = out.len();
        let mut n = 0;
        let mut pos = x.rec_at;
        let mut first = (x.rec, x.rec_at);
        let mut sink = |b: &[u8], pos: &mut usize| {
            for &c in b {
                if *pos >= at && *pos - at < out.len() {
                    out[*pos - at] = c;
                    n = n.max(*pos - at + 1);
                }
                *pos += 1;
            }
        };
        emit_from(store, &x, device, cx, x.rec, &mut |k, b| {
            // A record wholly before `at` moves the next start on.
            if pos < at && k > first.0 {
                first = (k, pos);
            }
            sink(b, &mut pos);
            pos < at + room
        });
        x.rec = first.0;
        x.rec_at = first.1;
        self.io.export = Some(x);
        n
    }

    // ---- The zones' screen --------------------------------------------

    pub(crate) fn zones_view(&mut self, f: &mut Frame, cx: &Ctx) {
        self.io.n_zones = 0;
        kit::large_title(
            f,
            cx,
            36,
            b"Time zone",
            b"Times are shown in this zone",
            tint::CALENDAR,
        );
        kit::panel(f, cx, FIELD);
        if self.io.typing {
            f.push(rp_app::Cmd::Outline {
                r: FIELD,
                color: kit::FOCUS,
            });
        }
        let q = &self.io.query[..self.io.query_n];
        if q.is_empty() {
            kit::text(
                f,
                FIELD.x + 14,
                FIELD.y + 25,
                cx.fonts.ui,
                DIM,
                b"A city: New York, London, Tokyo",
            );
        } else {
            kit::text(f, FIELD.x + 14, FIELD.y + 25, cx.fonts.ui, TEXT, q);
        }
        let utc =
            i64::from(cx.now.day) * 1440 + i64::from(cx.now.minute) - i64::from(cx.utc_offset);
        let mut y = FIELD.bottom() + 8;
        // First row: from the PC (the guess), then the matches.
        let r = Rect::new(MARGIN, y, WIDE, 44);
        kit::panel(f, cx, r);
        kit::text(f, r.x + 14, r.y + 28, cx.fonts.ui, TEXT, b"As your PC says");
        if self.io.guessed {
            kit::right(
                f,
                r.right() - 14,
                r.y + 28,
                cx.fonts.label,
                tint::CALENDAR,
                b"IN USE",
            );
        }
        self.io.zones_at[0] = (r, None);
        self.io.n_zones = 1;
        y += 48;
        let mut name = [0u8; 48];
        let mut skipped = 0;
        // Nothing typed: the zones most people pick, US first; else every
        // zone whose name holds what is typed.
        let matching = zone::all().filter(|z| {
            let mut name = [0u8; 48];
            let n = z.name(&mut name);
            q.is_empty()
                || name[..n].windows(q.len()).any(|w| {
                    w.iter()
                        .zip(q)
                        .all(|(a, b)| a.eq_ignore_ascii_case(b) || (*a == b'_' && *b == b' '))
                })
        });
        let mut common = zone::common();
        let mut matching = matching;
        let list: &mut dyn Iterator<Item = Zone> = if q.is_empty() {
            &mut common
        } else {
            &mut matching
        };
        for z in list {
            if skipped < self.io.zone_first {
                skipped += 1;
                continue;
            }
            if self.io.n_zones >= ZONE_ROWS {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, 44);
            kit::panel(f, cx, r);
            let l = zone_line(z, utc);
            kit::fitted(f, r.x + 14, r.y + 28, cx.fonts.ui, TEXT, l.get(), r.w - 120);
            let n = z.name(&mut name);
            let area = &name[..name[..n].iter().position(|&c| c == b'/').unwrap_or(0)];
            kit::right(f, r.right() - 14, r.y + 28, cx.fonts.label, DIM, area);
            if Some(z) == self.io.device && !self.io.guessed {
                kit::right(
                    f,
                    r.right() - 14,
                    r.y + 14,
                    cx.fonts.label,
                    tint::CALENDAR,
                    b"IN USE",
                );
            }
            self.io.zones_at[self.io.n_zones] = (r, Some(z));
            self.io.n_zones += 1;
            y += 48;
        }
        if self.io.zone_first > 0 {
            let more = digits(self.io.zone_first + 1);
            let mut b = Buf::<24>::new();
            b.s(b"from ").s(more.get());
            kit::right(
                f,
                FIELD.right() - 14,
                FIELD.y + 24,
                cx.fonts.label,
                DIM,
                b.get(),
            );
        }
    }

    fn zones_event(&mut self, e: Event, cx: &Ctx) -> Reply {
        match e {
            Event::Key(Key::SoftLeft) if self.io.typing => {
                self.io.typing = false;
                Reply::Redraw
            }
            Event::Key(Key::SoftLeft) => {
                self.view = View::Calendars;
                Reply::Redraw
            }
            Event::Char(c) if self.io.typing && self.io.query_n < self.io.query.len() => {
                self.io.query[self.io.query_n] = c;
                self.io.query_n += 1;
                self.io.zone_first = 0;
                Reply::Redraw
            }
            Event::Delete if self.io.typing => {
                self.io.query_n = self.io.query_n.saturating_sub(1);
                self.io.zone_first = 0;
                Reply::Redraw
            }
            Event::Done => {
                self.io.typing = false;
                Reply::Redraw
            }
            // Past the last row: the next page; before the first, back.
            Event::Key(Key::Down) if self.io.n_zones == ZONE_ROWS => {
                self.io.zone_first += ZONE_ROWS - 1;
                Reply::Redraw
            }
            Event::Key(Key::Up) if self.io.zone_first > 0 => {
                self.io.zone_first = self.io.zone_first.saturating_sub(ZONE_ROWS - 1);
                Reply::Redraw
            }
            Event::PenUp { x, y } if FIELD.contains_point(x, y) => {
                self.io.typing = true;
                Reply::Redraw
            }
            Event::PenUp { x, y } => {
                let Some(&(_, z)) = self.io.zones_at[..self.io.n_zones]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                else {
                    return Reply::Nothing;
                };
                self.keep_zone(cx, z);
                self.view = View::Calendars;
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    /// Zone `z` kept as the device's (`None`: back to the PC's guess).
    fn keep_zone(&mut self, cx: &Ctx, z: Option<Zone>) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let old = store
            .all(Kind::Data)
            .find(|r| r.text(tag::NAME) == b"zone")
            .map(|r| r.id);
        let mut name = [0u8; 48];
        let n = z.map_or(0, |z| z.name(&mut name));
        let ok = match (old, z) {
            (Some(id), Some(_)) => store.change(id, &[(tag::TEXT, &name[..n])]).is_ok(),
            (None, Some(_)) => store
                .put(
                    Kind::Data,
                    None,
                    &[(tag::NAME, b"zone"), (tag::TEXT, &name[..n])],
                )
                .is_ok(),
            (Some(id), None) => store.remove(id).is_ok(),
            (None, None) => true,
        };
        if ok {
            self.seen = None;
        }
    }
}

/// An appointment's zone for the file: its own, or (a floating time) the
/// device's, written as a zone so another calendar puts it at the same
/// hour; all-day ones have none.
fn zone_of(r: &rp_pim::store::Rec<'_>, device: Option<Zone>) -> Short<40> {
    let mut s = Short::new();
    if r.num(tag::ALL_DAY) == Some(1) {
        return s;
    }
    let own = r.text(tag::ZONE);
    if own.is_empty() {
        if let Some(d) = device {
            let mut b = [0u8; 40];
            let n = d.name(&mut b);
            s.push(&b[..n]);
        }
    } else {
        s.push(own);
    }
    s
}

/// "Calendar 2026-11-23.ics".
fn export_name(day: i32, out: &mut Buf<40>) {
    let d = date::date_of(day);
    out.s(b"Calendar ").num(d.year.unsigned_abs()).s(b"-");
    if d.month < 10 {
        out.s(b"0");
    }
    out.num(u32::from(d.month)).s(b"-");
    if d.day < 10 {
        out.s(b"0");
    }
    out.num(u32::from(d.day)).s(b".ics");
}

/// The whole file to `sink`.
fn emit(
    store: &rp_pim::store::Store<'_>,
    x: &Export,
    device: Option<Zone>,
    cx: &Ctx,
    sink: &mut dyn FnMut(&[u8]),
) {
    emit_from(store, x, device, cx, 0, &mut |_, b| {
        sink(b);
        true
    });
}

/// The file from record `from` on (`from` 0: with its head), each piece
/// handed to `sink` with the record it belongs to (0 the head, k+1 record
/// k, then the tail), until `sink` answers false.
fn emit_from(
    store: &rp_pim::store::Store<'_>,
    x: &Export,
    device: Option<Zone>,
    cx: &Ctx,
    from: usize,
    sink: &mut dyn FnMut(usize, &[u8]) -> bool,
) {
    let mut go = true;
    if from == 0 {
        let mut names = [&b""[..]; 8];
        for (n, z) in names.iter_mut().zip(&x.zones[..x.n_zones]) {
            *n = z.get();
        }
        rp_pim::ics::head(&names[..x.n_zones], |b| go = go && sink(0, b));
    }
    let stamp = i64::from(cx.now.day) * 1440 + i64::from(cx.now.minute) - i64::from(cx.utc_offset);
    for (k, r) in store.all(Kind::Appointment).enumerate() {
        if !go {
            return;
        }
        if k + 1 < from {
            continue;
        }
        let start = r.num(tag::START).unwrap_or(0);
        let all_day = r.num(tag::ALL_DAY) == Some(1);
        let zone = zone_of(&r, device);
        let rule = r
            .num(tag::REPEAT)
            .and_then(|v| rp_pim::repeat::Rule::unpack(v, date::day_of(start)));
        let e = rp_pim::ics::Out {
            id: r.id,
            title: r.text(tag::TITLE),
            place: r.text(tag::PLACE),
            note: r.text(tag::NOTE),
            start,
            minutes: r.num(tag::MINUTES).unwrap_or(60),
            all_day,
            rule,
            zone: zone.get(),
            alarm: r.num(tag::REMIND),
        };
        rp_pim::ics::event(&e, stamp, |b| go = go && sink(k + 1, b));
    }
    if go {
        rp_pim::ics::tail(|b| {
            let _ = sink(usize::MAX, b);
        });
    }
}

/// The editor's line under its start: where a time kept in another zone
/// is from ("9:00 AM in New York").
pub fn zone_note(name: &[u8], wall: u32) -> Option<Buf<48>> {
    let z = zone::find(name)?;
    let mut place = [0u8; 32];
    let n = z.place(&mut place);
    let mut b = Buf::new();
    b.s(b"KEPT AS ")
        .s(clock((wall % date::DAY_MIN) as u16).get())
        .s(b" IN ")
        .s(&place[..n]);
    Some(b)
}

#[cfg(test)]
impl Calendar {
    pub(crate) fn keep_zone_for_test(&mut self, cx: &Ctx, z: Option<Zone>) {
        self.keep_zone(cx, z);
    }
    pub(crate) fn io_row(&self, k: usize) -> Option<Rect> {
        self.io.rows_at[..self.io.n_at].get(k).map(|r| r.0)
    }
    pub(crate) fn zone_row(&self, k: usize) -> Option<Rect> {
        self.io.zones_at[..self.io.n_zones].get(k).map(|r| r.0)
    }
}

/// The export's key to the card (the tests tap it).
#[cfg(test)]
pub const TO_CARD_FOR_TEST: Rect = TO_CARD;
