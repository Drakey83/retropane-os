//! Contacts, version 2 ([contacts-rule0.md](../../../docs/design/contacts-rule0.md)):
//! an app program ([`rp_app::Program`]).
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 listed "Last, First"
//! rows and kept numbers exactly as typed; iOS and Google give everyone a
//! face (a photo or coloured initials), format numbers as typed and
//! suggest addresses. Ours does all three on the X51v: a favourites grid
//! and avatars painted once into the picture store, numbers formatted for
//! their country while typing from a few bytes a country, capitals at each
//! word in names and addresses, and addresses suggested over whatever
//! connection there is.

#![no_std]
#![forbid(unsafe_code)]

use rp_app::Text;
use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Event, Field, Frame, Key, MARGIN, Pic, Program, Rect, Reply, W, WIDE};
use rp_pim::contact::{Address, Day};
use rp_pim::phone;
use rp_pim::store::{Kind, tag};

pub mod form;
pub mod io;
pub mod map;
pub mod photo;

pub use form::EditField;
use form::{EXTRAS, Extra, FORM, SLOTS};

/// People a view holds.
pub const MAX: usize = 48;
/// Pictures drawn and kept at once (a page of the list, its favourites
/// and a card).
const FACES: usize = 20;
/// The PC's pictures a page of the chooser offers.
const PICKS: usize = 8;
/// A picture's pages in the store (as a note's ink: `Kind::Ink` records
/// naming the person), and the bytes one holds.
const PHOTO_PAGES: usize = 6;
/// The card's targets past what it copies: the avatar (the chooser).
const PICTURE: u8 = 9;
/// The card's Map key (group E: the address on the radar's map).
const MAP_KEY: u8 = 6;
/// The chooser's keys past its rows, and its card pictures' rows.
const MORE: u8 = 100;
const REMOVE: u8 = 101;
const CARD_PIC: u8 = 200;

/// What the network is asked for, or was.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    Nothing,
    /// The street's suggestions.
    Addr,
    /// The PC's pictures from this one on.
    List(u8),
    /// The PC's picture of this number.
    Photo(u8),
    /// The place of the address on the map page (Photon's first answer).
    Geo,
}

/// One person as the views show them (copied from the store's record).
#[derive(Clone, Copy, Debug)]
pub struct Person {
    /// The store's record.
    pub id: u32,
    /// First name.
    pub first: Text<24>,
    /// Last name.
    pub last: Text<24>,
    /// Company.
    pub company: Text<32>,
    /// Job title.
    pub job: Text<32>,
    /// Phones: a label and the number as kept (E.164).
    pub phones: [(&'static [u8], Text<20>); 3],
    /// E-mail.
    pub email: Text<40>,
    /// The address as the card shows it: the street, and the city line
    /// ("Seattle, WA 98101").
    pub address: [Text<40>; 2],
    /// A favourite.
    pub favourite: bool,
}

impl Person {
    /// A person from the store's record.
    fn of(r: &rp_pim::store::Rec<'_>) -> Self {
        let a = Address::of(r.text(tag::ADDRESS));
        let mut city = [0u8; 96];
        let n = a.city_text(&mut city);
        let (a0, a1) = (a.street, &city[..n]);
        Self {
            id: r.id,
            first: Text::of(r.text(tag::FIRST)),
            last: Text::of(r.text(tag::LAST)),
            company: Text::of(r.text(tag::COMPANY)),
            job: Text::of(r.text(tag::JOB)),
            phones: [
                (b"MOBILE", Text::of(r.text(tag::MOBILE))),
                (b"HOME", Text::of(r.text(tag::HOME))),
                (b"WORK", Text::of(r.text(tag::WORK))),
            ],
            email: Text::of(r.text(tag::EMAIL)),
            address: [Text::of(a0), Text::of(a1)],
            favourite: r.num(tag::FAVOURITE) == Some(1),
        }
    }
}

/// The editor's avatar (a touch on a kept person's chooses a picture).
const EDIT_FACE: Rect = Rect::new(W - MARGIN - 52, 44, 40, 40);

/// The views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// Everyone, the favourites on top.
    List,
    /// One person.
    Card(u8),
    /// A new person being typed: the field typed.
    Edit(EditField),
    /// One person's picture being chosen.
    Photo(u8),
    /// One person's address on the map (map.rs).
    Map(u8),
    /// Import, export and the doubles ([`io`]).
    Tools,
    /// A .vcf file chosen to import.
    Files,
    /// A double offered as one.
    Merge,
}

/// Where the form's first row lies.
const FORM_Y: u16 = 36 + kit::TITLE_BAND + 6;

/// A line of a person's card: its label, its text (and a second line for
/// an address).
#[derive(Clone, Copy, Debug)]
struct Detail {
    label: Text<24>,
    value: Text<64>,
    more: Text<64>,
}

impl Detail {
    const EMPTY: Self = Self {
        label: Text::EMPTY,
        value: Text::EMPTY,
        more: Text::EMPTY,
    };
}

/// The card's lines at most.
const DETAILS: usize = 20;

/// The list's search field and letter strip.
/// The search field, the home's first line (design-language.md version
/// 3: people first; the count is its hint, no title band above it).
const SEARCH: Rect = Rect::new(MARGIN, 44, WIDE - 60, 40);
const STRIP: Rect = Rect::new(W - MARGIN - 22, 120, 26, 470);

/// Whether `hay` holds `needle`, ignoring case.
fn holds(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// Whether person `p` matches the search `q`: any part of the name, the
/// company, the e-mail, the city, the initials ("gh"), or the digits of a
/// number however it was typed ("617 555").
fn matches(p: &Person, q: &[u8]) -> bool {
    let mut nm = [0u8; 64];
    let nn = name(p, &mut nm);
    if holds(&nm[..nn], q).is_some()
        || holds(p.company.get(), q).is_some()
        || holds(p.email.get(), q).is_some()
        || holds(p.address[1].get(), q).is_some()
    {
        return true;
    }
    let (ini, n) = kit::initials(&nm[..nn]);
    if q.len() == n && ini[..n].eq_ignore_ascii_case(q) {
        return true;
    }
    let mut qd = [0u8; 24];
    let mut qn = 0;
    for &c in q.iter().filter(|c| c.is_ascii_digit()) {
        if qn < qd.len() {
            qd[qn] = c;
            qn += 1;
        }
    }
    qn >= 3
        && q.iter()
            .all(|c| c.is_ascii_digit() || *c == b' ' || *c == b'-')
        && p.phones.iter().any(|(_, num)| {
            let digits: [u8; 24] = {
                let mut d = [0u8; 24];
                let mut k = 0;
                for &c in num.get().iter().filter(|c| c.is_ascii_digit()) {
                    if k < d.len() {
                        d[k] = c;
                        k += 1;
                    }
                }
                d
            };
            let dn = num
                .get()
                .iter()
                .filter(|c| c.is_ascii_digit())
                .count()
                .min(24);
            holds(&digits[..dn], &qd[..qn]).is_some()
        })
}

/// An address suggested: by the network (Photon) or, offline, from the
/// addresses already kept.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    /// The street line.
    pub street: Text<48>,
    /// City, state and postal code.
    pub city: Text<48>,
    /// Where it is, when the service said (millionths of a degree): kept
    /// with the address when it is picked (map.rs).
    pub at: Option<(i32, i32)>,
}

/// Where suggestion `k` lies under the street field (the form folded
/// above it).
const fn suggestion_row(k: u16) -> Rect {
    // Under the street's row, the second of the form while it is typed.
    Rect::new(MARGIN, FORM_Y + 2 * 48 + 2 + k * 46, WIDE, 46)
}

/// The Contacts program.
pub struct Contacts {
    /// The map page while it is up (map.rs).
    map: Option<map::MapPage>,
    /// The place of the suggestion picked in the editor, with the street
    /// and city it came with (kept only if the address saved is that one),
    /// and the address the editor opened with.
    geo_pick: Option<((i32, i32), u32)>,
    addr_was: u32,
    people: [Option<Person>; MAX],
    /// The store's change count the people were copied at.
    seen: Option<u32>,
    view: View,
    /// Typed text per editor field ([`FORM`]'s order), the person being
    /// changed (none: a new one), the favourite switch, Delete asked once,
    /// and the form's rows on screen.
    form: [[u8; 40]; SLOTS],
    form_n: [usize; SLOTS],
    /// The extra phones and e-mails being edited, and a day typed that
    /// isn't one (its row says so).
    extras: [Extra; EXTRAS],
    n_extras: usize,
    bad_day: Option<EditField>,
    editing: Option<u32>,
    fav: bool,
    confirm_delete: bool,
    /// Done was pressed with no name typed: the form says so.
    need_name: bool,
    form_at: [(Rect, EditField); 16],
    n_form: usize,
    /// Suggestions for the street (from the network service).
    places: [Option<Place>; 5],
    /// The list's page: the first person of it (among those shown), the
    /// pages before it, the one after its last, how many are shown; where
    /// the stylus went down (a drag turns the page).
    first_row: u8,
    pages: [u8; 12],
    n_pages: usize,
    next_row: u8,
    total: u8,
    down_y: Option<u16>,
    /// The card's targets (what each copies: 0-2 a phone, 3 the e-mail,
    /// 4 the address, 5 the Copy key), and the one just copied with the
    /// ticks its mark shows.
    card_at: [(Rect, u8); 24],
    n_card: usize,
    copied: Option<(u8, u8)>,
    /// The card's lines (read from the record as it is drawn), the first
    /// shown (a card of more lines than fit goes on over the page), and
    /// whether more follow.
    details: [Detail; DETAILS],
    n_details: usize,
    card_first: u8,
    card_more: bool,
    /// The card and its fields as the store takes them: kept here, not on
    /// a frame (4 KB together, the stack rule's whole).
    work: rp_pim::vcard::Card,
    scratch: rp_pim::vcard::Scratch,
    /// Import, export, duplicates and pictures from the card ([`io`]).
    io: io::Io,
    /// The doubles to be counted at the next draw.
    recount: bool,
    /// The JPEG reader of a picture being read, a vCard picture's bytes on
    /// their way to it, and the second card of a merge.
    dec: rp_pim::jpeg::Decoder,
    pic_buf: [u8; 2048],
    other: rp_pim::vcard::Card,
    /// A person's picture written as a JPEG for an export.
    jpeg_buf: [u8; io::JPEG_ROOM],
    /// The street text last asked about, and whether the last answer came
    /// from the device's own addresses (no connection).
    asked: Text<64>,
    offline: bool,
    /// First names kept that start as the one typed (the strip).
    names: [Text<24>; 3],
    /// The list's search typed (and being typed), the letter the list
    /// starts at, the letter under the pen on the strip (shown large), and
    /// the rows on screen with the people they open.
    query: [u8; 32],
    query_n: usize,
    searching: bool,
    jump: Option<u8>,
    bubble: Option<u8>,
    rows_at: [(Rect, u8); 16],
    n_rows: usize,
    /// The D-pad's place among the people drawn.
    focus: kit::Focus,
    /// The avatars: one disc per identity colour at 40 px, the card's 96 px.
    discs: [Option<Pic>; 8],
    /// The favourites' big glossy discs, one an identity colour.
    bigs: [Option<Pic>; 8],
    big: Option<Pic>,
    painted: bool,
    /// The people with a picture kept (by their place in `people`).
    has_photo: [bool; MAX],
    /// Pictures drawn (the person, the size, the picture), the next to
    /// give up its place.
    faces: [(u32, u8, Option<Pic>); FACES],
    face_next: usize,
    /// The chooser: the PC's pictures offered (their numbers and names),
    /// whether more follow, the first one's number, the rows and keys on
    /// screen, what to ask next, what was asked, and its line of status.
    picks: [(u8, Text<28>); PICKS],
    n_picks: usize,
    more: bool,
    pick_from: u8,
    pick_at: [(Rect, u8); PICKS + 6],
    n_pick_at: usize,
    want: Want,
    asked_for: Want,
    photo_note: &'static [u8],
}

/// "(206) 555-0142" for a kept "+12065550142" (the home plan: the US).
fn shown(e164: &[u8]) -> phone::Formatted {
    let home = phone::plan(*b"US").unwrap_or(&phone::PLANS[0]);
    // A home number is shown nationally, others internationally.
    if e164.starts_with(b"+1") {
        phone::format(&e164[2..], home)
    } else {
        phone::format(e164, home)
    }
}

fn name(p: &Person, out: &mut [u8; 64]) -> usize {
    let mut n = 0;
    for &c in p
        .first
        .get()
        .iter()
        .chain(if p.first.is_empty() {
            &b""[..]
        } else {
            &b" "[..]
        })
        .chain(p.last.get())
    {
        if n < out.len() {
            out[n] = c;
            n += 1;
        }
    }
    n
}

/// Every field's start, made as a value or straight into a slot
/// (`rp_hal_mem::in_place!`): one list for [`Contacts::new`] and
/// [`Contacts::new_in`].
macro_rules! contacts_start {
    ($($how:tt)*) => {
        rp_hal_mem::in_place!($($how)* Contacts {
            map: None,
            geo_pick: None,
            addr_was: 0,
            people: @each None,
            seen: None,
            view: View::List,
            form: [[0; 40]; SLOTS],
            form_n: [0; SLOTS],
            extras: [Extra::EMPTY; EXTRAS],
            n_extras: 0,
            bad_day: None,
            editing: None,
            fav: false,
            confirm_delete: false,
            need_name: false,
            form_at: [(Rect::new(0, 0, 0, 0), EditField::First); 16],
            n_form: 0,
            places: [None; 5],
            first_row: 0,
            pages: [0; 12],
            n_pages: 0,
            next_row: 0,
            total: 0,
            down_y: None,
            card_at: [(Rect::new(0, 0, 0, 0), 0); 24],
            n_card: 0,
            copied: None,
            details: @each Detail::EMPTY,
            n_details: 0,
            card_first: 0,
            card_more: false,
            work: rp_pim::vcard::Card::new(),
            scratch: rp_pim::vcard::Scratch::new(),
            io: io::Io::new(),
            recount: false,
            dec: rp_pim::jpeg::Decoder::new(),
            pic_buf: [0; 2048],
            jpeg_buf: @each 0,
            other: rp_pim::vcard::Card::new(),
            asked: Text::EMPTY,
            offline: false,
            names: [Text::EMPTY; 3],
            query: [0; 32],
            query_n: 0,
            searching: false,
            jump: None,
            bubble: None,
            rows_at: [(Rect::new(0, 0, 0, 0), 0); 16],
            n_rows: 0,
            focus: kit::Focus::default(),
            discs: [None; 8],
            bigs: [None; 8],
            big: None,
            painted: false,
            has_photo: @each false,
            faces: @each (0, 0, None),
            face_next: 0,
            picks: [(0, Text::of(b"")); PICKS],
            n_picks: 0,
            more: false,
            pick_from: 0,
            pick_at: [(Rect::new(0, 0, 0, 0), 0); PICKS + 6],
            n_pick_at: 0,
            want: Want::Nothing,
            asked_for: Want::Nothing,
            photo_note: b"",
        })
    };
}

impl Contacts {
    /// No one yet.
    #[must_use]
    pub fn new() -> Self {
        contacts_start!(value)
    }

    /// [`Contacts::new`] made straight into `slot` (the program's, in
    /// `.bss`), a field at a time: made on the stack and moved it was a
    /// 31 KB frame (the stack rule, P6: 4 KB).
    pub fn new_in(slot: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        contacts_start!(slot,)
    }

    /// The people made again from the store when its records changed.
    fn refresh(&mut self, cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        if self.seen == Some(store.changes()) {
            return;
        }
        self.seen = Some(store.changes());
        self.people = [None; MAX];
        for (slot, r) in self.people.iter_mut().zip(store.all(Kind::Contact)) {
            *slot = Some(Person::of(&r));
        }
        // Who has a picture, and the pictures drawn before made again
        // (one may have changed).
        self.has_photo = [false; MAX];
        for r in store.all(Kind::Ink) {
            if let Some(parent) = r.num(tag::PARENT)
                && let Some(k) = self
                    .people
                    .iter()
                    .position(|p| p.is_some_and(|p| p.id == parent))
            {
                self.has_photo[k] = true;
            }
        }
        for face in &mut self.faces {
            if let (Some(pic), Some(pics)) = (face.2.take(), cx.pics) {
                pics.free(pic);
            }
            face.0 = 0;
        }
    }

    /// Person `id`'s picture pages in the store, in the order written.
    fn photo_pages(store: &rp_pim::store::Store<'_>, id: u32) -> ([u32; PHOTO_PAGES], usize) {
        let mut ids = [0u32; PHOTO_PAGES];
        let mut n = 0;
        for r in store.all(Kind::Ink) {
            if r.num(tag::PARENT) == Some(id) && n < PHOTO_PAGES {
                let at = ids[..n].iter().position(|&i| i > r.id).unwrap_or(n);
                ids.copy_within(at..n, at + 1);
                ids[at] = r.id;
                n += 1;
            }
        }
        (ids, n)
    }

    /// Person `id`'s picture taken out of the store.
    fn photo_drop(cx: &Ctx, id: u32) {
        let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok()) else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let (ids, n) = Self::photo_pages(store, id);
        for &page in &ids[..n] {
            let _ = store.remove(page);
        }
    }

    /// The PC's answer `raw` kept as person `id`'s picture (in place of
    /// any before): `None` if it is not a picture, else whether the store
    /// took it all.
    fn photo_keep(&mut self, cx: &Ctx, id: u32, raw: &[u8]) -> Option<bool> {
        if raw.len() != photo::RAW {
            return None;
        }
        Self::photo_drop(cx, id);
        let kept = {
            let mut guard = cx.pim.and_then(|c| c.try_borrow_mut().ok())?;
            let store = guard.as_mut()?;
            photo::encode(raw, |page| {
                store
                    .put(
                        Kind::Ink,
                        None,
                        &[(tag::PARENT, &id.to_le_bytes()), (tag::INK, page)],
                    )
                    .is_ok()
            })?
        };
        if !kept {
            // Never half a picture.
            Self::photo_drop(cx, id);
        }
        self.seen = None;
        Some(kept)
    }

    /// Person `id`'s picture as a disc `size` pixels across, drawn once
    /// and kept while there is room (`None`: no picture, or no room).
    fn face(&mut self, cx: &Ctx, id: u32, size: u8) -> Option<Pic> {
        let k = self
            .people
            .iter()
            .position(|p| p.is_some_and(|p| p.id == id))?;
        if !self.has_photo[k] {
            return None;
        }
        if let Some(face) = self.faces.iter().find(|f| f.0 == id && f.1 == size) {
            return face.2;
        }
        let pics = cx.pics?;
        let guard = cx.pim?.try_borrow().ok()?;
        let store = guard.as_ref()?;
        // Its pages where they lie in the store.
        let (ids, n) = Self::photo_pages(store, id);
        let mut pages: [&[u8]; PHOTO_PAGES] = [&[]; PHOTO_PAGES];
        for (page, &id) in pages.iter_mut().zip(&ids[..n]) {
            *page = store.get(id)?.text(tag::INK);
        }
        let slot = self.face_next % FACES;
        if let Some(old) = self.faces[slot].2.take() {
            pics.free(old);
        }
        self.faces[slot] = (0, 0, None);
        let pic = pics.alloc(u16::from(size), u16::from(size))?;
        let mut drawn = false;
        pics.write(pic, |px| {
            drawn = photo::decode(&pages[..n], usize::from(size), px);
        });
        if !drawn {
            pics.free(pic);
            return None;
        }
        self.faces[slot] = (id, size, Some(pic));
        self.face_next += 1;
        Some(pic)
    }

    /// A person's avatar at `at`: their picture, or the disc with their
    /// initials.
    #[allow(clippy::too_many_arguments)]
    fn avatar(
        &mut self,
        f: &mut Frame,
        cx: &Ctx,
        id: Option<u32>,
        disc: Option<Pic>,
        at: (u16, u16),
        size: u8,
        font: &'static rp_app::Font,
        name: &[u8],
    ) {
        if let Some(pic) = id.and_then(|id| self.face(cx, id, size)) {
            f.push(rp_app::Cmd::Pic {
                x: at.0,
                y: at.1 as i16,
                pic,
            });
        } else {
            kit::avatar(f, disc, at.0, at.1, u16::from(size), font, name);
        }
    }

    /// The chooser of person `i`'s picture, asking the PC for its list.
    fn open_photo(&mut self, i: u8) {
        self.go(View::Photo(i));
        self.n_picks = 0;
        self.more = false;
        self.pick_from = 0;
        self.want = Want::List(0);
        self.photo_note = b"Looking on the card and your PC...";
        self.ask_card_pics();
    }

    /// What is typed in field `at` (nothing for a switch or a key).
    fn val(&self, at: EditField) -> &[u8] {
        match at {
            EditField::Extra(k) => self
                .extras
                .get(usize::from(k))
                .map_or(&[][..], |e| &e.value[..e.value_n]),
            EditField::Label(k) => self
                .extras
                .get(usize::from(k))
                .map_or(&[][..], |e| &e.label[..e.label_n]),
            _ => at
                .slot()
                .map_or(&[][..], |k| &self.form[k][..self.form_n[k]]),
        }
    }

    /// Field `at` set to `s` (as much as it holds).
    fn put(&mut self, at: EditField, s: &[u8]) {
        match at {
            EditField::Extra(k) => {
                if let Some(e) = self.extras.get_mut(usize::from(k)) {
                    e.value_n = s.len().min(40);
                    e.value[..e.value_n].copy_from_slice(&s[..e.value_n]);
                }
            }
            EditField::Label(k) => {
                if let Some(e) = self.extras.get_mut(usize::from(k)) {
                    e.label_n = s.len().min(24);
                    e.label[..e.label_n].copy_from_slice(&s[..e.label_n]);
                }
            }
            _ => {
                if let Some(k) = at.slot() {
                    let n = s.len().min(FORM[k].2);
                    self.form[k][..n].copy_from_slice(&s[..n]);
                    self.form_n[k] = n;
                }
            }
        }
    }

    /// A character typed into field `at` (none past its room).
    fn push_char(&mut self, at: EditField, c: u8) {
        let mut b = [0u8; 41];
        let v = self.val(at);
        let n = v.len();
        b[..n].copy_from_slice(v);
        b[n] = c;
        self.put(at, &b[..=n]);
    }

    /// The last character of field `at` taken off.
    fn pop_char(&mut self, at: EditField) {
        let mut b = [0u8; 40];
        let v = self.val(at);
        let n = v.len().saturating_sub(1);
        b[..n].copy_from_slice(&v[..n]);
        self.put(at, &b[..n]);
    }

    /// The form's rows in order: the common fields, each extra phone and
    /// e-mail under its kind with a key to add another, the address's
    /// parts, the days, the switch, and Delete for a person already kept.
    fn fields(&self) -> ([EditField; 32], usize) {
        use EditField as E;
        let mut out = [E::First; 32];
        let mut n = 0;
        let mut add = |f: E| {
            out[n] = f;
            n += 1;
        };
        for f in [
            E::First,
            E::Last,
            E::Company,
            E::Job,
            E::Mobile,
            E::Home,
            E::Work,
        ] {
            add(f);
        }
        let room = self.n_extras < EXTRAS;
        for (k, e) in self.extras[..self.n_extras].iter().enumerate() {
            if e.phone {
                add(E::Extra(k as u8));
            }
        }
        if room {
            add(E::AddPhone);
        }
        add(E::Email);
        for (k, e) in self.extras[..self.n_extras].iter().enumerate() {
            if !e.phone {
                add(E::Extra(k as u8));
            }
        }
        if room {
            add(E::AddEmail);
        }
        for f in [
            E::Street,
            E::City,
            E::Region,
            E::Postal,
            E::Country,
            E::Birthday,
            E::Anniversary,
            E::Website,
            E::Favourite,
        ] {
            add(f);
        }
        if self.editing.is_some() {
            add(E::Delete);
        }
        (out, n)
    }

    /// The row a field is typed on (an extra's label is its row's).
    const fn row_of(at: EditField) -> EditField {
        match at {
            EditField::Label(k) => EditField::Extra(k),
            f => f,
        }
    }

    /// `view` shown, the D-pad's focus and the marks of the last one gone.
    fn go(&mut self, view: View) {
        self.view = view;
        self.need_name = false;
        self.focus.clear();
        self.confirm_delete = false;
        self.copied = None;
        self.card_first = 0;
    }

    /// The editor on an empty form: a new person.
    fn open_new(&mut self) {
        self.form_n = [0; SLOTS];
        self.n_extras = 0;
        self.bad_day = None;
        self.editing = None;
        self.fav = false;
        self.places = [None; 5];
        self.addr_was = self.address_key();
        self.geo_pick = None;
        self.go(View::Edit(EditField::First));
    }

    /// The editor on person `i`: every field as kept.
    fn open_edit(&mut self, i: u8, cx: &Ctx) {
        use EditField as E;
        let Some(p) = self.people.get(usize::from(i)).copied().flatten() else {
            return;
        };
        {
            let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
                return;
            };
            let Some(r) = guard.as_ref().and_then(|s| s.get(p.id)) else {
                return;
            };
            // Into the app's own card: 2 KB, not on a frame.
            rp_pim::vcard::read_into(&r, &mut self.work);
        }
        self.form_n = [0; SLOTS];
        self.n_extras = 0;
        self.bad_day = None;
        let (first, last, company, job) = (
            self.work.first,
            self.work.last,
            self.work.company,
            self.work.job,
        );
        self.put(E::First, first.get());
        self.put(E::Last, last.get());
        self.put(E::Company, company.get());
        self.put(E::Job, job.get());
        for k in 0..self.work.n_phones {
            let ph = self.work.phones[k];
            let slot = match ph.label.get() {
                b"MOBILE" => Some(E::Mobile),
                b"HOME" => Some(E::Home),
                b"WORK" => Some(E::Work),
                _ => None,
            };
            match slot {
                Some(f) if self.val(f).is_empty() => self.put(f, ph.value.get()),
                _ if self.n_extras < EXTRAS => {
                    self.extras[self.n_extras] = Extra::of(true, ph.label.get(), ph.value.get());
                    self.n_extras += 1;
                }
                _ => {}
            }
        }
        for k in 0..self.work.n_emails {
            let e = self.work.emails[k];
            if k == 0 {
                self.put(E::Email, e.value.get());
            } else if self.n_extras < EXTRAS {
                self.extras[self.n_extras] = Extra::of(false, e.label.get(), e.value.get());
                self.n_extras += 1;
            }
        }
        for (k, f) in [E::Street, E::City, E::Region, E::Postal, E::Country]
            .into_iter()
            .enumerate()
        {
            let part = self.work.address[k];
            self.put(f, part.get());
        }
        for (d, f) in [
            (self.work.birthday, E::Birthday),
            (self.work.anniversary, E::Anniversary),
        ] {
            if let Some(d) = d {
                let (b, n) = form::day_text(d, cx.us);
                self.put(f, &b[..n]);
            }
        }
        let url = self.work.url;
        self.put(E::Website, url.get());
        self.addr_was = self.address_key();
        self.geo_pick = None;
        self.editing = Some(p.id);
        self.fav = p.favourite;
        self.places = [None; 5];
        self.go(View::Edit(E::First));
    }

    /// The form kept in the store: a new person, or the one being changed
    /// (a field emptied is taken off the record; fields the form doesn't
    /// show, a note, a picture, are left as they are). Numbers are kept as
    /// E.164 and formatted for the region when shown; a day typed that
    /// isn't one keeps nothing and says so on its row.
    fn keep(&mut self, cx: &Ctx) -> bool {
        use EditField as E;
        if self.val(E::First).is_empty()
            && self.val(E::Last).is_empty()
            && self.val(E::Company).is_empty()
        {
            // Said, not refused in silence: back to the name's row.
            self.need_name = true;
            self.view = View::Edit(E::First);
            return false;
        }
        let mut days = [None; 2];
        for (k, f) in [E::Birthday, E::Anniversary].into_iter().enumerate() {
            match form::day_typed(self.val(f), cx.us) {
                Some(d) => days[k] = d,
                None => {
                    self.bad_day = Some(f);
                    self.view = View::Edit(f);
                    return false;
                }
            }
        }
        let home = phone::plan(*b"US").unwrap_or(&phone::PLANS[0]);
        Self::form_card(
            &self.form,
            &self.form_n,
            &self.extras[..self.n_extras],
            days,
            &mut self.work,
        );
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        // The note isn't the form's: kept as it is.
        if let Some(r) = self.editing.and_then(|id| store.get(id)) {
            self.work.note.push(r.text(tag::NOTE));
        }
        self.scratch.fill(&self.work, home);
        let f = self.scratch.fields(&self.work);
        let one = 1u32.to_le_bytes();
        let mut all = [(0u8, &[][..]); rp_pim::vcard::FIELDS + 1];
        all[..rp_pim::vcard::FIELDS].copy_from_slice(&f);
        all[rp_pim::vcard::FIELDS] = (tag::FAVOURITE, if self.fav { &one } else { &[] });
        let kept = match self.editing {
            Some(id) => store.change(id, &all).ok().map(|_| id),
            None => store.put(Kind::Contact, None, &all).ok(),
        };
        if let Some(id) = kept {
            self.keep_geo(store, id);
        }
        let ok = kept.is_some();
        if ok {
            self.seen = None;
            self.form_n = [0; SLOTS];
            self.n_extras = 0;
            self.editing = None;
        }
        ok
    }

    /// The address's place with the person (map.rs): the suggestion's when
    /// the address saved is the one picked; taken off when the address
    /// changed any other way (a stale pin is worse than none: the map page
    /// asks again); left as it is when the address didn't change.
    fn keep_geo(&mut self, store: &mut rp_pim::store::Store<'static>, id: u32) {
        let now = self.address_key();
        let bytes;
        let geo: &[u8] = match self.geo_pick {
            Some((at, picked)) if picked == now => {
                bytes = map::geo_bytes(at);
                &bytes
            }
            _ if self.addr_was == now => return,
            _ => &[],
        };
        let _ = store.change(id, &[(tag::GEO, geo)]);
        self.geo_pick = None;
    }

    /// The map page for person `i`: their kept place, or the address asked
    /// about once (map.rs).
    fn open_map(&mut self, i: u8, cx: &Ctx) {
        let Some(p) = self.people.get(usize::from(i)).copied().flatten() else {
            return;
        };
        let at = cx.pim.and_then(|c| c.try_borrow().ok()).and_then(|g| {
            g.as_ref()
                .and_then(|s| s.get(p.id).and_then(|r| map::geo_of(r.text(tag::GEO))))
        });
        self.map = Some(map::MapPage::new(p.id, at));
        self.go(View::Map(i));
    }

    /// The place found for the map page's address (Photon's first answer
    /// with one), kept on the record.
    fn geo_found(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
        let Some(page) = self.map.as_mut() else {
            return Reply::Nothing;
        };
        if !ok {
            page.status = map::Status::Offline;
            return Reply::Redraw;
        }
        let mut found = [rp_pim::net::Place::EMPTY; 5];
        let n = rp_pim::net::photon(body, &mut found);
        let Some(at) = found[..n].iter().find_map(|p| p.at) else {
            page.status = map::Status::NotFound;
            return Reply::Redraw;
        };
        page.at = Some(at);
        page.status = map::Status::Drawing;
        let id = page.id;
        if let Some(cell) = cx.pim
            && let Ok(mut g) = cell.try_borrow_mut()
            && let Some(store) = g.as_mut()
        {
            let _ = store.change(id, &[(tag::GEO, &map::geo_bytes(at))]);
            self.seen = None;
        }
        Reply::Redraw
    }

    /// The previews' map page: person `i`, the picture lent.
    pub fn mock_map(&mut self, i: u8, at: (i32, i32), pic: Option<Pic>) {
        let id = self
            .people
            .get(usize::from(i))
            .copied()
            .flatten()
            .map_or(0, |p| p.id);
        let mut page = map::MapPage::new(id, Some(at));
        if let Some(pic) = pic {
            page.pic = Some((pic, page.zoom));
            page.status = map::Status::Shown;
        }
        self.map = Some(page);
        self.view = View::Map(i);
    }

    /// The map page's taps and keys: zoom (the keys, or the D-pad's up and
    /// down), the address copied (its card, or the centre key), back.
    fn map_event(&mut self, e: Event, i: u8, cx: &Ctx) -> Reply {
        let Some(page) = self.map.as_mut() else {
            return Reply::Nothing;
        };
        let [plus, minus] = map::ZOOM_KEYS;
        let by = match e {
            Event::PenUp { x, y } if plus.contains_point(x, y) => 1,
            Event::PenUp { x, y } if minus.contains_point(x, y) => -1,
            Event::Key(Key::Up) => 1,
            Event::Key(Key::Down) => -1,
            Event::Tick if page.copied > 0 => {
                page.copied -= 1;
                return if page.copied > 0 {
                    Reply::Animate
                } else {
                    Reply::Redraw
                };
            }
            Event::PenUp { x, y } if map::ADDRESS_CARD.contains_point(x, y) => {
                return self.map_copy(i, cx);
            }
            Event::Key(Key::Enter) => return self.map_copy(i, cx),
            Event::Key(Key::SoftLeft) => {
                self.map = None;
                self.go(View::Card(i));
                return Reply::Redraw;
            }
            _ => return Reply::Nothing,
        };
        if page.zoom_by(by) {
            Reply::Redraw
        } else {
            Reply::Nothing
        }
    }

    /// The map page's address onto the clipboard, with its mark for a
    /// second (as the card's lines).
    fn map_copy(&mut self, i: u8, cx: &Ctx) -> Reply {
        if let Some(p) = self.people.get(usize::from(i)).copied().flatten() {
            *cx.clip.borrow_mut() = Text::of(map::address_line(&p.address).get());
        }
        if let Some(page) = self.map.as_mut() {
            page.copied = 32;
        }
        Reply::Animate
    }

    /// The address as the form holds it, every part (street, city, region,
    /// postal code, country), as one number: whether it changed since it
    /// was opened or picked.
    fn address_key(&self) -> u32 {
        use EditField as E;
        let mut h = 0x811C_9DC5u32;
        for f in [E::Street, E::City, E::Region, E::Postal, E::Country] {
            for &b in self.val(f).iter().chain(b"\n") {
                h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
            }
        }
        h
    }

    /// The form as a card, into `c` (the app's own: not on a frame).
    #[inline(never)]
    fn form_card(
        form: &[[u8; 40]; SLOTS],
        form_n: &[usize; SLOTS],
        extras: &[Extra],
        days: [Option<Day>; 2],
        c: &mut rp_pim::vcard::Card,
    ) {
        use EditField as E;
        let v = |f: E| f.slot().map_or(&[][..], |k| &form[k][..form_n[k]]);
        c.clear();
        c.first.push(v(E::First));
        c.last.push(v(E::Last));
        c.company.push(v(E::Company));
        c.job.push(v(E::Job));
        for (f, label) in [
            (E::Mobile, &b"MOBILE"[..]),
            (E::Home, b"HOME"),
            (E::Work, b"WORK"),
        ] {
            if !v(f).is_empty() {
                let _ = c.add_phone(label, v(f));
            }
        }
        if !v(E::Email).is_empty() {
            let _ = c.add_email(b"Other", v(E::Email));
        }
        for e in extras.iter().filter(|e| e.value_n > 0) {
            let _ = if e.phone {
                c.add_phone(e.label(), &e.value[..e.value_n])
            } else {
                c.add_email(e.label(), &e.value[..e.value_n])
            };
        }
        for (k, f) in [E::Street, E::City, E::Region, E::Postal, E::Country]
            .into_iter()
            .enumerate()
        {
            c.address[k].push(v(f));
        }
        c.birthday = days[0];
        c.anniversary = days[1];
        c.url.push(v(E::Website));
    }

    /// The person being changed taken out of the store.
    fn delete(&mut self, cx: &Ctx) -> bool {
        let Some(id) = self.editing else { return false };
        let done = cx
            .pim
            .and_then(|c| c.try_borrow_mut().ok())
            .is_some_and(|mut g| g.as_mut().is_some_and(|s| s.remove(id).is_ok()));
        if done {
            Self::photo_drop(cx, id);
            self.seen = None;
            self.editing = None;
        }
        done
    }

    /// The list from its first page (the search or the letter changed).
    fn first_page(&mut self) {
        self.first_row = 0;
        self.n_pages = 0;
    }

    /// The list's next page, if there is one.
    fn page_on(&mut self) -> bool {
        if self.next_row >= self.total || self.next_row <= self.first_row {
            return false;
        }
        if self.n_pages < self.pages.len() {
            self.pages[self.n_pages] = self.first_row;
            self.n_pages += 1;
        }
        self.first_row = self.next_row;
        true
    }

    /// The page before, if there is one.
    fn page_back(&mut self) -> bool {
        if self.n_pages == 0 {
            if self.first_row == 0 {
                return false;
            }
            self.first_row = 0;
        } else {
            self.n_pages -= 1;
            self.first_row = self.pages[self.n_pages];
        }
        true
    }

    /// The text target `kind` of the card copies: a line (its two lines
    /// joined for an address), or (the Copy key) the first number.
    fn card_text(&self, p: &Person, kind: u8) -> Text<96> {
        if let Some(d) = self.details[..self.n_details].get(usize::from(kind).wrapping_sub(10)) {
            let mut b = [0u8; 96];
            let mut n = 0;
            for &c in d
                .value
                .get()
                .iter()
                .chain(if d.more.is_empty() { &b""[..] } else { b", " })
                .chain(d.more.get())
            {
                if n < b.len() {
                    b[n] = c;
                    n += 1;
                }
            }
            return Text::of(&b[..n]);
        }
        p.phones
            .iter()
            .find(|ph| !ph.1.is_empty())
            .map_or(Text::EMPTY, |ph| Text::of(shown(ph.1.get()).get()))
    }

    /// Shows `view` (the tools and the doubles counted at the next draw,
    /// where the store is).
    pub fn show(&mut self, view: View) {
        self.view = view;
        self.recount = matches!(view, View::Tools | View::Merge);
    }

    /// An extra phone or e-mail added to the editor and typed in (the
    /// previews).
    pub fn typed_extra(&mut self, phone: bool, label: &[u8], value: &[u8]) {
        if self.n_extras < EXTRAS {
            self.extras[self.n_extras] = Extra::of(phone, label, value);
            self.n_extras += 1;
        }
    }

    /// Field `at` typed (the previews).
    pub fn typed_field(&mut self, at: EditField, text: &[u8]) {
        self.put(at, text);
    }

    /// A day typed that isn't one, said on its row (the previews).
    pub fn typed_bad_day(&mut self, at: EditField, text: &[u8]) {
        self.put(at, text);
        self.bad_day = Some(at);
    }

    /// Types into the editor's fields (the previews).
    pub fn typed(&mut self, first: &[u8], mobile: &[u8], street: &[u8]) {
        self.put(EditField::First, first);
        self.put(EditField::Mobile, mobile);
        self.put(EditField::Street, street);
    }

    /// The chooser's page from the PC's answer (`from`: the number its
    /// list starts at; the previews call it too).
    pub fn set_pictures_from(&mut self, body: &[u8], from: u8) {
        // "number|name" a line; one more than a page: more follow.
        let mut n = 0;
        self.more = false;
        for line in body.split(|&c| c == b'\n') {
            let Some(bar) = line.iter().position(|&c| c == b'|') else {
                continue;
            };
            let number = line[..bar].iter().try_fold(0u8, |v, &c| {
                c.is_ascii_digit()
                    .then(|| v.checked_mul(10)?.checked_add(c - b'0'))
                    .flatten()
            });
            let Some(number) = number else { continue };
            if n == PICKS {
                self.more = true;
                break;
            }
            self.picks[n] = (number, Text::of(&line[bar + 1..]));
            n += 1;
        }
        self.n_picks = n;
        self.photo_note = match (n, from) {
            (0, 0) => b"No pictures in Pictures on your PC",
            (0, _) => b"No more pictures",
            _ => b"Choose one",
        };
    }

    /// Suggestions read from an address service's answer (Photon's
    /// GeoJSON): the network's answer, and the previews' canned one.
    pub fn set_places_from(&mut self, answer: &[u8]) {
        let mut found = [rp_pim::net::Place::EMPTY; 5];
        let n = rp_pim::net::photon(answer, &mut found);
        self.places = [None; 5];
        for (slot, p) in self.places.iter_mut().zip(&found[..n]) {
            *slot = Some(Place {
                street: Text::of(p.street.get()),
                city: Text::of(p.city.get()),
                at: p.at,
            });
        }
        self.offline = false;
    }

    /// Offline: the addresses already kept whose street starts as typed.
    fn places_kept(&mut self, cx: &Ctx) {
        self.places = [None; 5];
        self.offline = true;
        let typed = Text::<48>::of(self.val(EditField::Street));
        let typed = typed.get();
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        let mut k = 0;
        for r in store.all(Kind::Contact) {
            let a = Address::of(r.text(tag::ADDRESS));
            let st = a.street;
            if k < 5 && st.len() >= typed.len() && st[..typed.len()].eq_ignore_ascii_case(typed) {
                let mut city = [0u8; 96];
                let n = a.city_text(&mut city);
                self.places[k] = Some(Place {
                    street: Text::of(st),
                    city: Text::of(&city[..n]),
                    at: map::geo_of(r.text(tag::GEO)),
                });
                k += 1;
            }
        }
    }

    /// Suggestion `k` taken: street and city filled, the list gone.
    fn take_place(&mut self, k: usize) {
        let Some(p) = self.places.get(k).copied().flatten() else {
            return;
        };
        self.put(EditField::Street, p.street.get());
        let a = Address::city_line(p.city.get());
        self.put(EditField::City, a.city);
        self.put(EditField::Region, a.region);
        self.put(EditField::Postal, a.postal);
        // The suggestion's place, kept if the address saved is this one.
        self.geo_pick = p.at.map(|at| (at, self.address_key()));
        self.places = [None; 5];
        self.asked = Text::of(p.street.get());
    }

    /// The first names kept that start as the one typed (the strip).
    fn names_like(&mut self, cx: &Ctx) {
        self.names = [Text::EMPTY; 3];
        let typed = Text::<48>::of(self.val(EditField::First));
        let typed = typed.get();
        if typed.is_empty() {
            return;
        }
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        let mut k = 0;
        for r in store.all(Kind::Contact) {
            let f = r.text(tag::FIRST);
            if k < 3
                && f.len() > typed.len()
                && f[..typed.len()].eq_ignore_ascii_case(typed)
                && !self.names[..k].iter().any(|n| n.get() == f)
            {
                self.names[k] = Text::of(f);
                k += 1;
            }
        }
    }

    fn sorted(&self) -> ([u8; MAX], usize) {
        let mut idx = [0u8; MAX];
        let mut n = 0;
        for (i, p) in self.people.iter().enumerate() {
            if p.is_some() {
                idx[n] = i as u8;
                n += 1;
            }
        }
        let key = |i: &u8| {
            self.people[usize::from(*i)].map_or([0u8; 24], |p| {
                let mut k = [0u8; 24];
                let l = p.last.get();
                k[..l.len()].copy_from_slice(l);
                k
            })
        };
        idx[..n].sort_unstable_by_key(key);
        (idx, n)
    }

    fn paint(&mut self, cx: &Ctx) {
        if self.painted {
            return;
        }
        if let Some(p) = cx.pics {
            for (k, d) in self.discs.iter_mut().enumerate() {
                *d = kit::gloss_disc(p, 40, kit::IDENTITY[k]);
            }
            for (k, d) in self.bigs.iter_mut().enumerate() {
                *d = kit::gloss_disc(p, 64, kit::IDENTITY[k]);
            }
            self.big = kit::disc(p, 96, tint::CONTACTS, None);
        }
        self.painted = true;
    }

    /// A favourite's big disc, by the name's colour.
    fn big_for(&self, name: &[u8]) -> Option<Pic> {
        let c = kit::colour_of(name);
        let k = kit::IDENTITY.iter().position(|&x| x == c).unwrap_or(0);
        self.bigs[k]
    }

    fn disc_for(&self, name: &[u8]) -> Option<Pic> {
        let c = kit::colour_of(name);
        let k = kit::IDENTITY.iter().position(|&x| x == c).unwrap_or(0);
        self.discs[k]
    }

    fn list(&mut self, f: &mut Frame, cx: &Ctx) {
        self.n_rows = 0;
        let (all, total) = self.sorted();
        // The search keeps who matches; the strip's letter starts the list.
        let query = self.query;
        let q = &query[..self.query_n];
        let mut idx = [0u8; MAX];
        let mut n = 0;
        for &i in &all[..total] {
            let Some(p) = self.people[usize::from(i)] else {
                continue;
            };
            let initial = p
                .last
                .get()
                .first()
                .copied()
                .unwrap_or(b'#')
                .to_ascii_uppercase();
            if (q.is_empty() || matches(&p, q)) && self.jump.is_none_or(|j| initial >= j) {
                idx[n] = i;
                n += 1;
            }
        }
        if usize::from(self.first_row) >= n {
            self.first_row = 0;
            self.n_pages = 0;
        }
        self.total = n as u8;
        let mut s = [0u8; 24];
        let mut k = 0;
        let mut num = n;
        let mut digits = [0u8; 4];
        let mut d = digits.len();
        loop {
            d -= 1;
            digits[d] = b'0' + (num % 10) as u8;
            num /= 10;
            if num == 0 {
                break;
            }
        }
        for &c in digits[d..].iter().chain(b" people") {
            s[k] = c;
            k += 1;
        }
        // Search, its hint the count; the tools' key beside it.
        let field = SEARCH;
        kit::panel(f, cx, field);
        self.tools_key(f, cx);
        if self.searching {
            f.push(rp_app::Cmd::Outline {
                r: field,
                color: kit::FOCUS,
            });
        }
        if q.is_empty() {
            let mut hint = [0u8; 32];
            let mut hn = 0;
            for &c in b"Search ".iter().chain(&s[..k]) {
                hint[hn] = c;
                hn += 1;
            }
            kit::text(f, field.x + 16, field.y + 27, cx.fonts.ui, DIM, &hint[..hn]);
        } else {
            kit::text(f, field.x + 16, field.y + 27, cx.fonts.ui, TEXT, q);
        }
        let mut y = SEARCH.y + SEARCH.h + 6;
        // (A later page is the people only, as a search's is.)
        let searching = !q.is_empty() || self.jump.is_some() || self.first_row > 0;
        // The favourites grid: large discs with initials, three across
        // (not while searching: the matches come first).
        if !searching {
            kit::section(f, cx, y, b"FAVORITES", tint::CONTACTS);
            y += 22;
            let mut col = 0u16;
            for &i in &idx[..n] {
                let Some(p) = self.people[usize::from(i)] else {
                    continue;
                };
                if !p.favourite || col == 4 {
                    continue;
                }
                let cell = Rect::new(MARGIN + col * 104, y, 104, 100);
                let mut nm = [0u8; 64];
                let nn = name(&p, &mut nm);
                let disc = self.big_for(&nm[..nn]);
                self.avatar(
                    f,
                    cx,
                    Some(p.id),
                    disc,
                    (cell.x + 20, cell.y + 2),
                    64,
                    cx.fonts.title,
                    &nm[..nn],
                );
                kit::centred(f, cell, cell.y + 90, cx.fonts.ui, TEXT, p.first.get());
                // A favourite opens its card (drawn, so a target: the
                // version 2 favourites answered no tap).
                if self.n_rows < self.rows_at.len() {
                    self.rows_at[self.n_rows] = (cell, i);
                    self.n_rows += 1;
                }
                col += 1;
            }
            y += 104;
        }
        // Everyone, by letter.
        let initial_of = |p: &Person| {
            p.last
                .get()
                .first()
                .copied()
                .unwrap_or(b'#')
                .to_ascii_uppercase()
        };
        let mut at = usize::from(self.first_row);
        while at < n {
            let Some(first) = self.people[usize::from(idx[at])] else {
                at += 1;
                continue;
            };
            let l = initial_of(&first);
            // The letter's people, as many as fit under it.
            let mut group = 1;
            while at + group < n
                && self.people[usize::from(idx[at + group])].is_some_and(|p| initial_of(&p) == l)
            {
                group += 1;
            }
            if y + 22 + 54 > 596 {
                break;
            }
            kit::section(f, cx, y, &[l], DIM);
            y += 22;
            let fit = usize::from((596 - y) / 54).min(group);
            let card = Rect::new(MARGIN, y, WIDE - 24, fit as u16 * 54);
            kit::panel(f, cx, card);
            for (k, &i) in idx[at..at + fit].iter().enumerate() {
                let Some(p) = self.people[usize::from(i)] else {
                    continue;
                };
                let r = Rect::new(card.x, y + k as u16 * 54, card.w, 54);
                if k > 0 {
                    f.push(rp_app::Cmd::Fill {
                        r: Rect::new(r.x + 62, r.y, r.w - 76, 1),
                        color: kit::alpha(TEXT, 0x28),
                    });
                }
                if self.n_rows < self.rows_at.len() {
                    self.rows_at[self.n_rows] = (r, i);
                    self.n_rows += 1;
                }
                let mut nm = [0u8; 64];
                let nn = name(&p, &mut nm);
                let disc = self.disc_for(&nm[..nn]);
                self.avatar(
                    f,
                    cx,
                    Some(p.id),
                    disc,
                    (r.x + 12, r.y + 7),
                    40,
                    cx.fonts.ui,
                    &nm[..nn],
                );
                kit::fitted(f, r.x + 66, r.y + 24, cx.fonts.ui, TEXT, &nm[..nn], 220);
                // The match in the name, in the system accent.
                if let Some(m) = holds(&nm[..nn], q) {
                    let x = r.x + 66 + cx.fonts.ui.width(&nm[..m]);
                    kit::text(f, x, r.y + 24, cx.fonts.ui, kit::FOCUS, &nm[m..m + q.len()]);
                }
                let sub = if p.company.is_empty() {
                    p.email.get()
                } else {
                    p.company.get()
                };
                kit::fitted(f, r.x + 66, r.y + 44, cx.fonts.label, DIM, sub, 220);
                if !p.phones[0].1.is_empty() {
                    let ph = shown(p.phones[0].1.get());
                    kit::right(f, r.right() - 14, r.y + 24, cx.fonts.ui, DIM, ph.get());
                }
            }
            y += card.h + 4;
            // A letter with more people than fit goes on over the page
            // (they were cut off, out of the stylus's reach and the
            // D-pad's).
            at += fit;
            if fit < group {
                break;
            }
        }
        self.next_row = at as u8;
        if usize::from(self.next_row) < n || self.first_row > 0 {
            // Where this page sits: said in the search field's right.
            let mut s = [0u8; 16];
            let mut k = 0;
            let parts: [(usize, &[u8]); 3] = [
                (usize::from(self.first_row) + 1, b"-"),
                (at, b" of "),
                (n, b""),
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
                SEARCH.right() - 14,
                SEARCH.y + 26,
                cx.fonts.label,
                DIM,
                &s[..k],
            );
        }
        if n == 0 {
            kit::text(f, MARGIN + 8, y + 24, cx.fonts.ui, DIM, b"No one matches");
        }
        // The letter strip; the letter under the pen large beside it.
        let strip = STRIP;
        for (k, c) in (b'A'..=b'Z').enumerate() {
            let col = if self.jump == Some(c) {
                kit::FOCUS
            } else {
                DIM
            };
            kit::centred(
                f,
                Rect::new(strip.x, 0, strip.w, 1),
                strip.y + 12 + k as u16 * 18,
                cx.fonts.label,
                col,
                &[c],
            );
        }
        if let Some(c) = self.bubble {
            let r = Rect::new(
                strip.x - 96,
                strip.y + (u16::from(c - b'A')) * 18 - 30,
                80,
                72,
            );
            kit::key(f, cx, r, kit::KeyState::On);
            kit::centred(f, r, r.y + 50, cx.fonts.title, TEXT, &[c]);
        }
    }

    /// Person `i`'s picture chosen: the pictures in the photos folder on
    /// the PC, by name, a page at a time.
    fn chooser(&mut self, f: &mut Frame, cx: &Ctx, i: u8) {
        self.n_pick_at = 0;
        let Some(p) = self.people.get(usize::from(i)).copied().flatten() else {
            return;
        };
        let mut nm = [0u8; 64];
        let nn = name(&p, &mut nm);
        kit::panel(f, cx, Rect::new(8, 40, W - 16, 128));
        let big = self.big;
        self.avatar(
            f,
            cx,
            Some(p.id),
            big,
            (24, 56),
            96,
            cx.fonts.title,
            &nm[..nn],
        );
        kit::text(f, 136, 76, cx.fonts.label, tint::CONTACTS, b"PICTURE");
        kit::fitted(f, 136, 108, cx.fonts.title, TEXT, &nm[..nn], 320);
        kit::text(f, 136, 134, cx.fonts.ui, DIM, self.photo_note);
        kit::text(
            f,
            136,
            154,
            cx.fonts.label,
            DIM,
            b"FROM THE CARD AND THE PICTURES FOLDER ON YOUR PC",
        );
        let mut y = 178;
        // The card's pictures first (no dock needed), four at most.
        for k in 0..self.io.n_card_pics.min(4) {
            let r = Rect::new(MARGIN, y, WIDE, 40);
            kit::panel(f, cx, r);
            kit::fitted(
                f,
                r.x + 14,
                r.y + 26,
                cx.fonts.ui,
                TEXT,
                self.io.card_pics[k].1.get(),
                WIDE - 90,
            );
            kit::right(
                f,
                r.right() - 14,
                r.y + 26,
                cx.fonts.label,
                tint::CONTACTS,
                b"CARD",
            );
            self.pick_at[self.n_pick_at] = (r, CARD_PIC + k as u8);
            self.n_pick_at += 1;
            y += 44;
        }
        for k in 0..self.n_picks {
            if y + 44 > 540 {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, 40);
            kit::panel(f, cx, r);
            kit::fitted(
                f,
                r.x + 14,
                r.y + 26,
                cx.fonts.ui,
                TEXT,
                self.picks[k].1.get(),
                WIDE - 28,
            );
            self.pick_at[self.n_pick_at] = (r, k as u8);
            self.n_pick_at += 1;
            y += 44;
        }
        let mut key = |r: Rect, word: &[u8], kind: u8, this: &mut Self| {
            kit::key(f, cx, r, kit::KeyState::Rest);
            kit::centred(f, r, r.y + 23, cx.fonts.label, TEXT, word);
            this.pick_at[this.n_pick_at] = (r, kind);
            this.n_pick_at += 1;
        };
        if self.more {
            key(Rect::new(MARGIN, y + 2, 120, 36), b"MORE", MORE, self);
        }
        if self.has_photo[usize::from(i)] {
            key(
                Rect::new(W - MARGIN - 180, y + 2, 180, 36),
                b"REMOVE PICTURE",
                REMOVE,
                self,
            );
        }
    }

    /// Person `p`'s card's lines from their record: the numbers and
    /// e-mails (the three kept on their own, then each extra with its own
    /// label), the address, the days, the website, the note.
    fn details_of(&mut self, cx: &Ctx, p: &Person) {
        self.n_details = 0;
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return;
        };
        let Some(r) = guard.as_ref().and_then(|s| s.get(p.id)) else {
            return;
        };
        let add = |this: &mut Self, label: &[u8], value: &[u8], more: &[u8]| {
            if this.n_details < DETAILS && !value.is_empty() {
                // Labels shown in capitals, as the fixed ones are.
                let mut l = [0u8; 24];
                let n = label.len().min(24);
                for (o, c) in l.iter_mut().zip(&label[..n]) {
                    *o = c.to_ascii_uppercase();
                }
                this.details[this.n_details] = Detail {
                    label: Text::of(&l[..n]),
                    value: Text::of(value),
                    more: Text::of(more),
                };
                this.n_details += 1;
            }
        };
        for (label, number) in &p.phones {
            if !number.is_empty() {
                add(self, label, shown(number.get()).get(), b"");
            }
        }
        let more = r.text(tag::MORE);
        for e in rp_pim::contact::entries(more).filter(|e| e.kind == rp_pim::contact::Kind::Phone) {
            add(
                self,
                if e.label.is_empty() {
                    b"Other"
                } else {
                    e.label
                },
                shown(e.value).get(),
                b"",
            );
        }
        add(self, b"E-MAIL", p.email.get(), b"");
        for e in rp_pim::contact::entries(more).filter(|e| e.kind == rp_pim::contact::Kind::Email) {
            add(
                self,
                if e.label.is_empty() {
                    b"Other"
                } else {
                    e.label
                },
                e.value,
                b"",
            );
        }
        if !p.address[0].is_empty() || !p.address[1].is_empty() {
            let (a, b) = if p.address[0].is_empty() {
                (p.address[1], Text::EMPTY)
            } else {
                (p.address[0], p.address[1])
            };
            add(self, b"HOME", a.get(), b.get());
        }
        for (label, t) in [
            (&b"BIRTHDAY"[..], tag::BIRTHDAY),
            (b"ANNIV.", tag::ANNIVERSARY),
        ] {
            if let Some(d) = Day::of_field(r.text(t)) {
                let (b, n) = form::day_shown(d, cx.us);
                let mut more = [0u8; 24];
                let mut m = 0;
                // The age or the years at the next one.
                if let Some(y) = d.years_on(d.next_from(cx.now.day)) {
                    let mut put = |s: &[u8]| {
                        for &c in s {
                            if m < more.len() {
                                more[m] = c;
                                m += 1;
                            }
                        }
                    };
                    put(if t == tag::BIRTHDAY { b"turns " } else { b"" });
                    let mut digits = [0u8; 5];
                    let mut k = digits.len();
                    let mut v = y;
                    loop {
                        k -= 1;
                        digits[k] = b'0' + (v % 10) as u8;
                        v /= 10;
                        if v == 0 {
                            break;
                        }
                    }
                    put(&digits[k..]);
                    put(if t == tag::BIRTHDAY {
                        b" next"
                    } else {
                        b" years next"
                    });
                }
                add(self, label, &b[..n], &more[..m]);
            }
        }
        add(self, b"WEBSITE", r.text(tag::WEBSITE), b"");
        let note = r.text(tag::NOTE);
        let first = &note[..note.iter().position(|&c| c == b'\n').unwrap_or(note.len())];
        add(self, b"NOTE", first, b"");
    }

    fn card(&mut self, f: &mut Frame, cx: &Ctx, i: u8) {
        self.n_card = 0;
        let Some(p) = self.people.get(usize::from(i)).copied().flatten() else {
            return;
        };
        self.details_of(cx, &p);
        let mut nm = [0u8; 64];
        let nn = name(&p, &mut nm);
        // The band: the big avatar on the person's colour.
        let band = Rect::new(0, 36, W, 200);
        kit::panel(f, cx, Rect::new(8, band.y + 4, W - 16, band.h - 8));
        // (A touch on it chooses the person's picture.)
        let big = self.big;
        self.avatar(
            f,
            cx,
            Some(p.id),
            big,
            ((W - 96) / 2, 48),
            96,
            cx.fonts.title,
            &nm[..nn],
        );
        self.card_at[0] = (Rect::new((W - 96) / 2, 48, 96, 96), PICTURE);
        self.n_card = 1;
        let line = Rect::new(0, 0, W, 1);
        kit::centred(f, line, 180, cx.fonts.title, TEXT, &nm[..nn]);
        // "Programmer, Analytical Engines".
        let mut sub = [0u8; 72];
        let mut sn = 0;
        for &c in p
            .job
            .get()
            .iter()
            .chain(if p.job.is_empty() || p.company.is_empty() {
                &b""[..]
            } else {
                b", "
            })
            .chain(p.company.get())
        {
            if sn < sub.len() {
                sub[sn] = c;
                sn += 1;
            }
        }
        kit::centred(f, line, 204, cx.fonts.ui, DIM, &sub[..sn]);
        if p.favourite {
            kit::centred(f, line, 224, cx.fonts.label, tint::CONTACTS, b"FAVORITE");
        }
        let hit = |this: &mut Self, r: Rect, kind: u8| {
            if this.n_card < this.card_at.len() {
                this.card_at[this.n_card] = (r, kind);
                this.n_card += 1;
            }
        };
        let copied = self.copied.map(|c| c.0);
        // The action row. Copy and Map work (Map: the address on the
        // radar's map, map.rs); E-mail and To PC wait for the mail program
        // and the PC's side, and are drawn dim until then (a key that looks
        // live and does nothing is worse than one that says it is not
        // there yet).
        let mut y = 246;
        let has_number = p.phones.iter().any(|ph| !ph.1.is_empty());
        let has_address = !p.address[0].is_empty() || !p.address[1].is_empty();
        for (k, word) in [&b"E-mail"[..], b"Map", b"Copy", b"To PC"]
            .iter()
            .enumerate()
        {
            let r = Rect::new(MARGIN + k as u16 * 112, y, 104, 64);
            let live = (k == 2 && has_number) || (k == 1 && has_address);
            kit::key(
                f,
                cx,
                r,
                if live && copied == Some(5) {
                    kit::KeyState::On
                } else {
                    kit::KeyState::Rest
                },
            );
            let (word, col): (&[u8], _) = match (live, k == 2 && copied == Some(5)) {
                (true, true) => (b"Copied", TEXT),
                (true, false) => (word, TEXT),
                _ => (word, kit::alpha(DIM, 0x70)),
            };
            kit::centred(f, r, r.y + 40, cx.fonts.ui, col, word);
            if live {
                hit(self, r, if k == 1 { MAP_KEY } else { 5 });
            }
        }
        y += 76;
        // The details, a line each (an address two), as many as fit; a
        // card of more goes on over the page (a drag or the D-pad), as
        // the list does. A tap copies the line.
        if usize::from(self.card_first) >= self.n_details {
            self.card_first = 0;
        }
        let mut k = usize::from(self.card_first);
        while k < self.n_details {
            let d = self.details[k];
            let h = if d.more.is_empty() { 44 } else { 66 };
            if y + h > 590 {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, h);
            kit::panel(f, cx, r);
            kit::fitted(
                f,
                r.x + 14,
                r.y + 28,
                cx.fonts.label,
                DIM,
                d.label.get(),
                92,
            );
            let kind = 10 + k as u8;
            if copied == Some(kind) {
                kit::right(
                    f,
                    r.right() - 14,
                    r.y + 28,
                    cx.fonts.label,
                    tint::CONTACTS,
                    b"COPIED",
                );
            }
            kit::fitted(
                f,
                r.x + 110,
                r.y + 29,
                cx.fonts.ui,
                TEXT,
                d.value.get(),
                r.w - 190,
            );
            if !d.more.is_empty() {
                kit::fitted(
                    f,
                    r.x + 110,
                    r.y + 53,
                    cx.fonts.ui,
                    DIM,
                    d.more.get(),
                    r.w - 124,
                );
            }
            hit(self, r, kind);
            y += h + 2;
            k += 1;
        }
        self.card_more = k < self.n_details;
        if self.card_more || self.card_first > 0 {
            let mut b = [0u8; 16];
            let mut n = 0;
            for (v, sep) in [
                (usize::from(self.card_first) + 1, &b"-"[..]),
                (k, b" of "),
                (self.n_details, b""),
            ] {
                if v >= 10 {
                    b[n] = b'0' + (v / 10) as u8;
                    n += 1;
                }
                b[n] = b'0' + (v % 10) as u8;
                n += 1;
                for &c in sep {
                    b[n] = c;
                    n += 1;
                }
            }
            kit::right(f, W - MARGIN - 8, 240, cx.fonts.label, DIM, &b[..n]);
        }
    }

    /// The card's next page of lines, if more follow; the first page from
    /// a later one.
    fn card_page(&mut self, on: bool) -> bool {
        if on && self.card_more {
            // From the first line not shown.
            let shown = self.card_at[..self.n_card]
                .iter()
                .filter(|t| t.1 >= 10)
                .count() as u8;
            self.card_first += shown.max(1);
            true
        } else if !on && self.card_first > 0 {
            self.card_first = 0;
            true
        } else {
            false
        }
    }

    fn edit(&mut self, f: &mut Frame, cx: &Ctx, at: EditField) {
        self.n_form = 0;
        let mut y = kit::large_title(
            f,
            cx,
            36,
            if self.editing.is_some() {
                b"Edit contact"
            } else {
                b"New contact"
            },
            b"Typed fields format themselves",
            tint::CONTACTS,
        );
        let ini = Text::<48>::of(self.val(EditField::First));
        let disc = self.disc_for(ini.get());
        let id = self.editing;
        self.avatar(
            f,
            cx,
            id,
            disc,
            (EDIT_FACE.x, EDIT_FACE.y),
            40,
            cx.fonts.ui,
            ini.get(),
        );
        y += 6;
        let home = phone::plan(*b"US").unwrap_or(&phone::PLANS[0]);
        // The row typed is the form's second (the one before it leads, for
        // the place), so it and what follows it sit above the keyboard.
        let (order, n) = self.fields();
        let row = Self::row_of(at);
        let i = order[..n].iter().position(|&o| o == row).unwrap_or(0);
        for &fld in &order[i.saturating_sub(1)..n] {
            if y + 44 > 596 {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, 44);
            let on = fld == row;
            kit::panel(f, cx, r);
            if on {
                f.push(rp_app::Cmd::Outline {
                    r,
                    color: kit::FOCUS,
                });
            }
            let x = r.x + 110;
            let caret = |f: &mut Frame, x: u16, value: &[u8], room: u16| {
                let cw = cx.fonts.ui.width(value).min(room);
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(x + cw + 1, r.y + 10, 2, 24),
                    color: kit::FOCUS,
                });
            };
            match fld {
                EditField::Favourite => {
                    kit::text(f, r.x + 12, r.y + 28, cx.fonts.label, DIM, b"FAVORITE");
                    let (word, col): (&[u8], _) = if self.fav {
                        (b"Yes", tint::CONTACTS)
                    } else {
                        (b"No", DIM)
                    };
                    kit::text(f, x, r.y + 29, cx.fonts.ui, col, word);
                }
                EditField::Delete => {
                    let word: &[u8] = if self.confirm_delete {
                        b"Delete for good? Once more to delete"
                    } else {
                        b"Delete this contact"
                    };
                    kit::text(f, r.x + 12, r.y + 29, cx.fonts.ui, kit::LATE, word);
                }
                EditField::AddPhone | EditField::AddEmail => {
                    // A key in the row's place: "+ Add phone".
                    kit::text(
                        f,
                        r.x + 12,
                        r.y + 29,
                        cx.fonts.ui,
                        tint::CONTACTS,
                        if fld == EditField::AddPhone {
                            b"+  Add a phone"
                        } else {
                            b"+  Add an e-mail"
                        },
                    );
                    kit::right(
                        f,
                        r.right() - 14,
                        r.y + 28,
                        cx.fonts.label,
                        DIM,
                        b"WITH ITS OWN LABEL",
                    );
                }
                EditField::Extra(k) => {
                    let Some(e) = self.extras.get(usize::from(k)).copied() else {
                        continue;
                    };
                    // The label (tapped: typed, or one from the strip) in
                    // the tint; the value as a phone's or an e-mail's.
                    let typing_label = at == EditField::Label(k);
                    let mut label = [0u8; 24];
                    let shown_label = e.label();
                    for (o, c) in label.iter_mut().zip(shown_label) {
                        *o = c.to_ascii_uppercase();
                    }
                    let label = &label[..shown_label.len().min(24)];
                    if typing_label {
                        kit::fitted(
                            f,
                            r.x + 12,
                            r.y + 28,
                            cx.fonts.label,
                            TEXT,
                            &e.label[..e.label_n],
                            92,
                        );
                        caret(f, r.x + 12, &e.label[..e.label_n], 92);
                    } else {
                        kit::fitted(
                            f,
                            r.x + 12,
                            r.y + 28,
                            cx.fonts.label,
                            tint::CONTACTS,
                            label,
                            92,
                        );
                    }
                    let value = &e.value[..e.value_n];
                    let number = phone::format(value, home);
                    let value = if e.phone { number.get() } else { value };
                    if value.is_empty() {
                        let hint: &[u8] = if e.phone {
                            b"Number"
                        } else {
                            b"name@example.com"
                        };
                        kit::text(f, x, r.y + 29, cx.fonts.ui, kit::alpha(DIM, 0x90), hint);
                    } else {
                        kit::fitted(f, x, r.y + 29, cx.fonts.ui, TEXT, value, r.w - 124);
                    }
                    if on && !typing_label {
                        caret(f, x, value, r.w - 124);
                    }
                }
                _ => {
                    let Some(slot) = fld.slot() else { continue };
                    let (label, hint, _) = FORM[slot];
                    // The days' hint the region's way round.
                    let hint: &[u8] = match fld {
                        EditField::Birthday | EditField::Anniversary if !cx.us => {
                            b"DD/MM/YYYY, the year if known"
                        }
                        _ => hint,
                    };
                    let typed = self.val(fld);
                    let number = phone::format(typed, home);
                    let value =
                        if matches!(fld, EditField::Mobile | EditField::Home | EditField::Work) {
                            number.get()
                        } else {
                            typed
                        };
                    kit::fitted(f, r.x + 12, r.y + 28, cx.fonts.label, DIM, label, 96);
                    if value.is_empty() && self.need_name && fld == EditField::First {
                        kit::text(f, x, r.y + 29, cx.fonts.ui, kit::LATE, b"A name first");
                    } else if value.is_empty() {
                        kit::text(f, x, r.y + 29, cx.fonts.ui, kit::alpha(DIM, 0x90), hint);
                    } else {
                        kit::fitted(f, x, r.y + 29, cx.fonts.ui, TEXT, value, r.w - 124);
                    }
                    if self.bad_day == Some(fld) {
                        kit::right(
                            f,
                            r.right() - 14,
                            r.y + 28,
                            cx.fonts.label,
                            kit::LATE,
                            b"NOT A DAY",
                        );
                    }
                    if on {
                        caret(f, x, value, r.w - 124);
                    }
                }
            }
            if self.n_form < self.form_at.len() {
                self.form_at[self.n_form] = (r, fld);
                self.n_form += 1;
            }
            y += 48;
            if on && fld == EditField::Street && self.places.iter().any(Option::is_some) {
                // The suggestions sheet over the rest of the form.
                let n = self.places.iter().flatten().count() as u16;
                let sheet = Rect::new(MARGIN, y - 2, WIDE, 6 + n.min(3) * 46);
                kit::panel(f, cx, sheet);
                for (k, pl) in self.places.iter().flatten().take(3).enumerate() {
                    let ry = sheet.y + 4 + k as u16 * 46;
                    kit::text(f, sheet.x + 14, ry + 20, cx.fonts.ui, TEXT, pl.street.get());
                    kit::text(f, sheet.x + 14, ry + 40, cx.fonts.ui, DIM, pl.city.get());
                    if k == 0 && self.offline {
                        kit::right(f, sheet.right() - 12, ry + 20, cx.fonts.label, DIM, b"KEPT");
                    }
                    if k > 0 {
                        f.push(rp_app::Cmd::Fill {
                            r: Rect::new(sheet.x + 12, ry, sheet.w - 24, 1),
                            color: kit::alpha(TEXT, 0x30),
                        });
                    }
                }
                break;
            }
        }
    }
}

/// Contacts as it first opens (the program's entry and the PC's model).
#[must_use]
pub fn opened(_today: i32) -> Contacts {
    Contacts::new()
}

/// [`opened`] made straight into the program's slot.
pub fn opened_in(slot: &mut core::mem::MaybeUninit<Contacts>, _today: i32) -> &mut Contacts {
    Contacts::new_in(slot)
}

impl Default for Contacts {
    fn default() -> Self {
        Self::new()
    }
}

impl Program for Contacts {
    fn accent(&self) -> Option<rp_app::Argb> {
        Some(rp_app::kit::tint::CONTACTS)
    }

    fn title(&self) -> &'static str {
        "Contacts"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        match self.view {
            View::List => ("Back", "New"),
            View::Card(_) => ("Back", "Edit"),
            View::Edit(_) => ("Cancel", "Done"),
            View::Photo(_) | View::Map(_) => ("Back", ""),
            View::Tools | View::Files | View::Merge => ("Back", ""),
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        self.paint(cx);
        self.refresh(cx);
        if core::mem::take(&mut self.recount) {
            self.count_doubles(cx);
        }
        match self.view {
            View::List => {
                self.list(f, cx);
                self.focus.ring(f, self.targets());
            }
            View::Card(i) => {
                self.card(f, cx, i);
                self.focus.ring(f, self.targets());
            }
            View::Edit(at) => self.edit(f, cx, at),
            View::Photo(i) => {
                self.chooser(f, cx, i);
                self.focus.ring(f, self.targets());
            }
            View::Map(i) => {
                let Some(p) = self.people.get(usize::from(i)).copied().flatten() else {
                    return;
                };
                let mut nm = [0u8; 64];
                let nn = name(&p, &mut nm);
                if let Some(page) = self.map.as_mut() {
                    map::draw(
                        f,
                        cx,
                        page,
                        &nm[..nn],
                        p.address[0].get(),
                        p.address[1].get(),
                    );
                }
            }
            View::Tools => {
                self.tools_view(f, cx);
                self.focus.ring(f, self.targets());
            }
            View::Files => {
                self.files_view(f, cx);
                self.focus.ring(f, self.targets());
            }
            View::Merge => {
                self.merge_view(f, cx);
                self.focus.ring(f, self.targets());
            }
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        // The stylus takes over from the D-pad (the centre key's tap is
        // a lift only).
        if matches!(e, Event::PenDown { .. }) {
            self.focus.clear();
        }
        if let View::Map(i) = self.view
            && e != Event::Key(Key::App)
        {
            return self.map_event(e, i, cx);
        }
        // The D-pad (R203) on every screen. The list and a card: the
        // arrows walk what a stylus can tap (the search, the people; a
        // card's lines and its Copy key), the centre taps it; past the
        // list's last row or its first, the next page or the one before.
        // The editor: up and down move from row to row, the centre flips
        // the favourite switch and presses Delete. Windows Mobile's
        // Contacts moves a row at a time through one long list.
        if matches!(self.view, View::Tools | View::Files | View::Merge) {
            if let Event::Key(k @ (Key::Up | Key::Down | Key::Left | Key::Right)) = e {
                let mut focus = self.focus;
                let moved = focus.step(k, self.targets());
                self.focus = focus;
                return if moved { Reply::Redraw } else { Reply::Nothing };
            }
            if e == Event::Key(Key::Enter)
                && let Some((x, y)) = self.focus.point()
            {
                return self.io_event(Event::PenUp { x, y }, cx);
            }
            if e == Event::Key(Key::App) {
                self.go(View::List);
                return Reply::Redraw;
            }
            return self.io_event(e, cx);
        }
        if let Event::Key(k) = e
            && matches!(self.view, View::List | View::Card(_) | View::Photo(_))
            && !self.searching
        {
            let mut focus = self.focus;
            let moved = focus.step(k, self.targets());
            self.focus = focus;
            if moved {
                return Reply::Redraw;
            }
            if self.view == View::List
                && ((k == Key::Down && self.page_on()) || (k == Key::Up && self.page_back()))
            {
                self.focus.clear();
                return Reply::Redraw;
            }
            // A card of more lines than fit: its next page, or the first.
            if matches!(self.view, View::Card(_))
                && ((k == Key::Down && self.card_page(true))
                    || (k == Key::Up && self.card_page(false)))
            {
                self.focus.clear();
                return Reply::Redraw;
            }
            if k == Key::Enter
                && let Some((x, y)) = self.focus.point()
            {
                return self.event(Event::PenUp { x, y }, cx);
            }
        }
        if let (Event::Key(k @ (Key::Up | Key::Down | Key::Enter)), View::Edit(at)) = (e, self.view)
        {
            let (order, n) = self.fields();
            let row = Self::row_of(at);
            let i = order[..n].iter().position(|&o| o == row).unwrap_or(0);
            match k {
                Key::Enter => return self.press(at, cx),
                Key::Down if i + 1 < n => self.view = View::Edit(order[i + 1]),
                Key::Up if i > 0 => self.view = View::Edit(order[i - 1]),
                _ => return Reply::Nothing,
            }
            self.confirm_delete = false;
            return Reply::Redraw;
        }
        // The Contacts button, pressed in Contacts: back to the list.
        if e == Event::Key(Key::App) {
            self.searching = false;
            self.go(View::List);
            return Reply::Redraw;
        }
        match (e, self.view) {
            // The softkey Done keeps the person (as the keyboard's does).
            (Event::Key(Key::SoftRight), View::Edit(_)) => {
                if self.keep(cx) {
                    self.go(View::List);
                }
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::Card(i)) => {
                self.open_edit(i, cx);
                Reply::Redraw
            }
            (Event::PenDown { y, .. }, View::Card(_)) => {
                self.down_y = Some(y);
                Reply::Nothing
            }
            // A card's line, or its Copy key: onto the clipboard as it is
            // shown, with a mark for a second.
            (Event::PenUp { x, y }, View::Card(i)) => {
                // A drag up or down turns the card's page.
                if let Some(y0) = self.down_y.take()
                    && y0.abs_diff(y) > 48
                {
                    return if self.card_page(y < y0) {
                        Reply::Redraw
                    } else {
                        Reply::Nothing
                    };
                }
                let hit = self.card_at[..self.n_card]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                    .map(|t| t.1);
                if hit == Some(PICTURE) {
                    self.open_photo(i);
                    return Reply::Redraw;
                }
                if hit == Some(MAP_KEY) {
                    self.open_map(i, cx);
                    return Reply::Redraw;
                }
                if let Some(kind) = hit
                    && let Some(p) = self.people.get(usize::from(i)).copied().flatten()
                {
                    *cx.clip.borrow_mut() = Text::of(self.card_text(&p, kind).get());
                    self.copied = Some((kind, 32));
                    return Reply::Animate;
                }
                Reply::Nothing
            }
            (Event::Tick, View::Card(_)) => match self.copied {
                Some((kind, left)) if left > 1 => {
                    self.copied = Some((kind, left - 1));
                    Reply::Animate
                }
                Some(_) => {
                    self.copied = None;
                    Reply::Redraw
                }
                None => Reply::Nothing,
            },
            (Event::Key(Key::SoftLeft), View::List)
                if self.searching || self.query_n > 0 || self.jump.is_some() =>
            {
                self.searching = false;
                self.query_n = 0;
                self.jump = None;
                self.first_page();
                Reply::Redraw
            }
            (Event::Key(Key::SoftLeft), View::List) => Reply::Close,
            (Event::PenDown { x, y } | Event::PenMove { x, y }, View::List)
                if STRIP.contains_point(x, y) =>
            {
                let k = ((y.saturating_sub(STRIP.y)) / 18).min(25) as u8;
                let c = b'A' + k;
                self.bubble = Some(c);
                self.jump = Some(c);
                self.down_y = None;
                self.first_page();
                Reply::Redraw
            }
            (Event::PenDown { y, .. }, View::List) => {
                self.down_y = Some(y);
                Reply::Nothing
            }
            (Event::PenUp { x, y }, View::List) => {
                if self.bubble.take().is_some() {
                    return Reply::Redraw;
                }
                // A drag up or down turns the page.
                if let Some(y0) = self.down_y.take()
                    && y0.abs_diff(y) > 48
                {
                    let turned = if y < y0 {
                        self.page_on()
                    } else {
                        self.page_back()
                    };
                    return if turned {
                        Reply::Redraw
                    } else {
                        Reply::Nothing
                    };
                }
                if SEARCH.contains_point(x, y) {
                    self.searching = true;
                    return Reply::Redraw;
                }
                if io::TOOLS_KEY.contains_point(x, y) {
                    self.searching = false;
                    self.open_tools(cx);
                    return Reply::Redraw;
                }
                match self.rows_at[..self.n_rows]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                {
                    Some(&(_, i)) => {
                        self.go(View::Card(i));
                        Reply::Redraw
                    }
                    None => Reply::Nothing,
                }
            }
            (Event::Char(c), View::List) if self.searching && self.query_n < self.query.len() => {
                self.query[self.query_n] = c;
                self.query_n += 1;
                self.first_page();
                Reply::Redraw
            }
            (Event::Delete, View::List) if self.searching => {
                self.query_n = self.query_n.saturating_sub(1);
                self.first_page();
                Reply::Redraw
            }
            (Event::Done, View::List) if self.searching => {
                self.searching = false;
                Reply::Redraw
            }
            // The chooser: a picture's row asks the PC for it, MORE for
            // the next page of names, REMOVE takes the picture kept.
            (Event::PenUp { x, y }, View::Photo(i)) => {
                let hit = self.pick_at[..self.n_pick_at]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                    .map(|t| t.1);
                match hit {
                    Some(k) if k >= CARD_PIC => {
                        if let Some(p) = self.people.get(usize::from(i)).copied().flatten() {
                            self.pick_card_pic(cx, p.id, usize::from(k - CARD_PIC));
                        }
                    }
                    Some(MORE) => {
                        self.pick_from = self.pick_from.saturating_add(PICKS as u8);
                        self.want = Want::List(self.pick_from);
                        self.photo_note = b"Asking your PC for its pictures...";
                    }
                    Some(REMOVE) => {
                        if let Some(p) = self.people.get(usize::from(i)).copied().flatten() {
                            Self::photo_drop(cx, p.id);
                            self.seen = None;
                        }
                        self.go(View::Card(i));
                    }
                    Some(k) if usize::from(k) < self.n_picks => {
                        self.want = Want::Photo(self.picks[usize::from(k)].0);
                        self.photo_note = b"Getting the picture...";
                    }
                    _ => return Reply::Nothing,
                }
                Reply::Redraw
            }
            (Event::Key(Key::SoftLeft), View::Photo(i)) => {
                self.go(View::Card(i));
                Reply::Redraw
            }
            (Event::Key(Key::SoftLeft), _) => {
                self.go(View::List);
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::List) => {
                self.open_new();
                Reply::Redraw
            }
            (Event::Char(_) | Event::Delete, View::Edit(at)) => {
                if !at.typed() {
                    return Reply::Nothing;
                }
                match e {
                    Event::Char(c) => {
                        self.push_char(at, c);
                        self.need_name = false;
                    }
                    Event::Delete => self.pop_char(at),
                    _ => {}
                }
                if self.bad_day == Some(at) {
                    self.bad_day = None;
                }
                if at == EditField::First {
                    self.names_like(cx);
                }
                if at == EditField::Street && self.val(at).len() < 3 {
                    self.places = [None; 5];
                }
                Reply::Redraw
            }
            // A kept person's avatar in the editor: what is typed is
            // kept, then the chooser.
            (Event::PenUp { x, y }, View::Edit(_))
                if EDIT_FACE.contains_point(x, y) && self.editing.is_some() =>
            {
                let id = self.editing;
                if self.keep(cx) {
                    self.refresh(cx);
                    let i = self
                        .people
                        .iter()
                        .position(|p| p.map(|p| p.id) == id && id.is_some());
                    match i {
                        Some(i) => self.open_photo(i as u8),
                        None => self.go(View::List),
                    }
                }
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Edit(at)) => {
                // The street's suggestions lie over the rows under it.
                if at == EditField::Street
                    && let Some(k) = (0..3u16).find(|&k| {
                        suggestion_row(k).contains_point(x, y)
                            && self.places.get(usize::from(k)).is_some_and(Option::is_some)
                    })
                {
                    self.take_place(usize::from(k));
                    return Reply::Redraw;
                }
                // A row: typed from now, or (a switch, a key, Delete)
                // pressed; an extra's label, tapped, is typed (or one from
                // the strip over the keyboard).
                match self.form_at[..self.n_form]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                    .copied()
                {
                    Some((r, EditField::Extra(k))) if x < r.x + 100 => {
                        self.view = View::Edit(EditField::Label(k));
                        self.confirm_delete = false;
                        Reply::Redraw
                    }
                    Some((_, fld)) if fld == at || !fld.typed() => {
                        self.view = View::Edit(fld);
                        self.press(fld, cx)
                    }
                    Some((_, fld)) => {
                        self.view = View::Edit(fld);
                        self.confirm_delete = false;
                        Reply::Redraw
                    }
                    None => Reply::Nothing,
                }
            }
            (Event::Suggestion(k), View::Edit(EditField::Street)) => {
                self.take_place(usize::from(k));
                Reply::Redraw
            }
            (Event::Suggestion(k), View::Edit(EditField::Label(j))) => {
                if let Some(e) = self.extras.get(usize::from(j)).copied() {
                    let labels = if e.phone {
                        form::PHONE_LABELS
                    } else {
                        form::EMAIL_LABELS
                    };
                    self.put(EditField::Label(j), labels[usize::from(k).min(2)]);
                    self.view = View::Edit(EditField::Extra(j));
                }
                Reply::Redraw
            }
            // The label typed: on to its value.
            (Event::Done, View::Edit(EditField::Label(j))) => {
                self.view = View::Edit(EditField::Extra(j));
                Reply::Redraw
            }
            (Event::Suggestion(k), View::Edit(EditField::First)) => {
                let name = self.names[usize::from(k).min(2)];
                if !name.is_empty() {
                    self.put(EditField::First, name.get());
                    self.names = [Text::EMPTY; 3];
                }
                Reply::Redraw
            }
            // The keyboard's Done is done: the person is kept, as the
            // softkey's Done keeps them (it moved to the next field, and
            // on the last one did nothing: the owner on the X51v,
            // 2026-11-15, "Keyboard done doesnt do anything"). The rows are
            // chosen by a tap or the D-pad.
            (Event::Done, View::Edit(_)) => {
                if self.keep(cx) {
                    self.go(View::List);
                }
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        use EditField as E;
        if self.view == View::List && self.searching {
            return Some(Field::Line);
        }
        match self.view {
            View::Edit(
                E::First
                | E::Last
                | E::Company
                | E::Job
                | E::City
                | E::Region
                | E::Country
                | E::Label(_),
            ) => Some(Field::Name),
            View::Edit(E::Mobile | E::Home | E::Work) => Some(Field::Phone),
            View::Edit(E::Extra(k)) => Some(
                if self.extras.get(usize::from(k)).is_some_and(|e| e.phone) {
                    Field::Phone
                } else {
                    Field::Email
                },
            ),
            View::Edit(E::Email) => Some(Field::Email),
            View::Edit(E::Street | E::Postal) => Some(Field::Address),
            View::Edit(E::Birthday | E::Anniversary) => Some(Field::Number),
            View::Edit(E::Website) => Some(Field::Link),
            _ => None,
        }
    }

    fn suggestions(&self) -> [&[u8]; 3] {
        match self.view {
            View::Edit(EditField::First) => [
                self.names[0].get(),
                self.names[1].get(),
                self.names[2].get(),
            ],
            View::Edit(EditField::Street) => {
                let st = |k: usize| self.places[k].as_ref().map_or(&b""[..], |p| p.street.get());
                [st(0), st(1), st(2)]
            }
            View::Edit(EditField::Label(k)) => {
                if self.extras.get(usize::from(k)).is_some_and(|e| e.phone) {
                    form::PHONE_LABELS
                } else {
                    form::EMAIL_LABELS
                }
            }
            _ => [&[], &[], &[]],
        }
    }

    /// The street typed, once it has three characters and differs from
    /// the last one asked: Photon is asked over whatever connection there
    /// is; the answer's round trip paces the asking.
    fn map_ask(&self) -> Option<rp_app::MapAsk> {
        match self.view {
            View::Map(_) => self.map.as_ref().and_then(map::MapPage::ask),
            _ => None,
        }
    }

    fn map_answer(&mut self, ask: rp_app::MapAsk, map: Option<Pic>, _cx: &Ctx) -> Reply {
        let Some(page) = self.map.as_mut().filter(|p| p.ask() == Some(ask)) else {
            return Reply::Nothing;
        };
        page.pic = map.map(|m| (m, ask.zoom));
        page.status = if map.is_some() {
            map::Status::Shown
        } else {
            map::Status::NoMap
        };
        Reply::Redraw
    }

    fn net_ask(&mut self) -> Option<rp_app::NetAsk> {
        // The map page's address, asked once when its place isn't kept.
        if let (View::Map(i), Some(page)) = (self.view, self.map.as_mut())
            && page.status == map::Status::Finding
            && !page.asked
        {
            page.asked = true;
            let p = self.people.get(usize::from(i)).copied().flatten()?;
            self.asked_for = Want::Geo;
            return Some(rp_app::NetAsk {
                what: b"ADDR",
                arg: Text::of(map::address_line(&p.address).get()),
            });
        }
        if let Some(a) = self.io_net_ask() {
            return Some(a);
        }
        // The chooser's questions: the PC's pictures, or one of them.
        if let View::Photo(_) = self.view {
            let want = core::mem::replace(&mut self.want, Want::Nothing);
            let (what, n): (&'static [u8], u8) = match want {
                Want::List(from) => (b"PHOTOS", from),
                Want::Photo(n) => (b"PHOTO", n),
                _ => return None,
            };
            self.asked_for = want;
            let d = [b'0' + n / 100, b'0' + n / 10 % 10, b'0' + n % 10];
            let from = d.iter().position(|&c| c != b'0').unwrap_or(2);
            return Some(rp_app::NetAsk {
                what,
                arg: Text::of(&d[from..]),
            });
        }
        if self.view != View::Edit(EditField::Street) || self.val(EditField::Street).len() < 3 {
            return None;
        }
        let typed = Text::<48>::of(self.val(EditField::Street));
        let typed = typed.get();
        if self.asked.get() == typed {
            return None;
        }
        self.asked = Text::of(typed);
        self.asked_for = Want::Addr;
        Some(rp_app::NetAsk {
            what: b"ADDR",
            arg: Text::of(typed),
        })
    }

    fn net_answer(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
        if self.io.asking() {
            return self.io_answer(ok, body, cx);
        }
        match core::mem::replace(&mut self.asked_for, Want::Nothing) {
            Want::Geo => return self.geo_found(ok, body, cx),
            Want::List(from) => {
                if !ok {
                    self.photo_note = b"No connection (Wi-Fi or the dock)";
                    return Reply::Redraw;
                }
                self.set_pictures_from(body, from);
                return Reply::Redraw;
            }
            Want::Photo(_) => {
                let View::Photo(i) = self.view else {
                    return Reply::Nothing;
                };
                let id = self
                    .people
                    .get(usize::from(i))
                    .copied()
                    .flatten()
                    .map(|p| p.id);
                let kept = id
                    .filter(|_| ok)
                    .and_then(|id| self.photo_keep(cx, id, body));
                match kept {
                    Some(true) => self.go(View::Card(i)),
                    Some(false) => self.photo_note = b"No room left to keep it",
                    None => self.photo_note = b"The PC sent no picture",
                }
                return Reply::Redraw;
            }
            Want::Addr | Want::Nothing => {}
        }
        if ok {
            self.set_places_from(body);
            if self.places[0].is_none() {
                self.places_kept(cx);
            }
        } else {
            self.places_kept(cx);
        }
        Reply::Redraw
    }

    fn card_ask(&mut self) -> Option<rp_app::NetAsk> {
        self.io_card_ask()
    }

    fn net_body(&mut self, at: usize, out: &mut [u8], cx: &Ctx) -> usize {
        self.io_body(at, out, cx)
    }
}

impl Contacts {
    /// Everything on the screen a stylus can tap, for the D-pad: the
    /// list's search and people, a card's lines and its Copy key.
    fn targets(&self) -> impl Iterator<Item = Rect> + '_ {
        let list = self.view == View::List;
        let card = matches!(self.view, View::Card(_));
        let choosing = matches!(self.view, View::Photo(_));
        core::iter::once(SEARCH)
            .chain(core::iter::once(io::TOOLS_KEY))
            .filter(move |_| list)
            .chain(
                self.rows_at[..self.n_rows]
                    .iter()
                    .filter(move |_| list)
                    .map(|t| t.0),
            )
            .chain(
                self.card_at[..self.n_card]
                    .iter()
                    .filter(move |_| card)
                    .map(|t| t.0),
            )
            .chain(
                self.pick_at[..self.n_pick_at]
                    .iter()
                    .filter(move |_| choosing)
                    .map(|t| t.0),
            )
            .chain(self.io.targets(self.view))
    }

    /// A press on the editor's row `at` (a tap on it once chosen, or the
    /// centre key): the favourite switch flips, Delete asks and then
    /// deletes; a typed field only takes the typing.
    fn press(&mut self, at: EditField, cx: &Ctx) -> Reply {
        match at {
            EditField::Favourite => self.fav = !self.fav,
            EditField::Delete if self.confirm_delete => {
                if self.delete(cx) {
                    self.go(View::List);
                }
            }
            EditField::Delete => self.confirm_delete = true,
            // A new row under its kind, typed at once.
            EditField::AddPhone | EditField::AddEmail if self.n_extras < EXTRAS => {
                self.extras[self.n_extras] = Extra::of(at == EditField::AddPhone, b"", b"");
                self.view = View::Edit(EditField::Extra(self.n_extras as u8));
                self.n_extras += 1;
            }
            _ => {}
        }
        Reply::Redraw
    }
}

#[cfg(test)]
mod tests;
