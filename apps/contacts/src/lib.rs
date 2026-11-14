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
use rp_pim::phone;
use rp_pim::store::{Kind, tag};

/// People a view holds.
pub const MAX: usize = 48;

/// One person as the views show them (copied from the store's record).
#[derive(Clone, Copy, Debug)]
pub struct Person {
    /// First name.
    pub first: Text<24>,
    /// Last name.
    pub last: Text<24>,
    /// Company.
    pub company: Text<32>,
    /// Phones: a label and the number as kept (E.164).
    pub phones: [(&'static [u8], Text<20>); 3],
    /// E-mail.
    pub email: Text<40>,
    /// The address, as lines.
    pub address: [Text<40>; 2],
    /// A favourite.
    pub favourite: bool,
}

impl Person {
    /// A person from the store's record.
    fn of(r: &rp_pim::store::Rec<'_>) -> Self {
        let addr = r.text(tag::ADDRESS);
        let (a0, a1) = match addr.iter().position(|&c| c == b'\n') {
            Some(k) => (&addr[..k], &addr[k + 1..]),
            None => (addr, &b""[..]),
        };
        Self {
            first: Text::of(r.text(tag::FIRST)),
            last: Text::of(r.text(tag::LAST)),
            company: Text::of(r.text(tag::COMPANY)),
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

/// The views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// Everyone, the favourites on top.
    List,
    /// One person.
    Card(u8),
    /// A new person being typed: the field typed.
    Edit(EditField),
}

/// The field typed in the editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditField {
    /// The first name.
    First,
    /// The mobile number.
    Mobile,
    /// The street.
    Street,
}

/// The list's search field and letter strip.
const SEARCH: Rect = Rect::new(MARGIN, 36 + kit::TITLE_BAND + 4, WIDE - 24, 36);
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
}

/// Where suggestion `k` lies under the street field (the form folded
/// above it).
const fn suggestion_row(k: u16) -> Rect {
    Rect::new(
        MARGIN,
        36 + kit::TITLE_BAND + 6 + 40 + 48 + 2 + k * 46,
        WIDE,
        46,
    )
}

/// The Contacts program.
pub struct Contacts {
    people: [Option<Person>; MAX],
    /// The store's change count the people were copied at.
    seen: Option<u32>,
    view: View,
    /// Typed text per editor field.
    first: [u8; 32],
    first_n: usize,
    mobile: [u8; 24],
    mobile_n: usize,
    street: [u8; 48],
    street_n: usize,
    /// Suggestions for the street (from the network service).
    places: [Option<Place>; 5],
    /// The city line a suggestion filled.
    city: [u8; 48],
    city_n: usize,
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
    rows_at: [(Rect, u8); 12],
    n_rows: usize,
    /// The avatars: one disc per identity colour at 40 px, the card's 96 px.
    discs: [Option<Pic>; 8],
    big: Option<Pic>,
    painted: bool,
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

impl Contacts {
    /// No one yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            people: [None; MAX],
            seen: None,
            view: View::List,
            first: [0; 32],
            first_n: 0,
            mobile: [0; 24],
            mobile_n: 0,
            street: [0; 48],
            street_n: 0,
            places: [None; 5],
            city: [0; 48],
            city_n: 0,
            asked: Text::EMPTY,
            offline: false,
            names: [Text::EMPTY; 3],
            query: [0; 32],
            query_n: 0,
            searching: false,
            jump: None,
            bubble: None,
            rows_at: [(Rect::new(0, 0, 0, 0), 0); 12],
            n_rows: 0,
            discs: [None; 8],
            big: None,
            painted: false,
        }
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
    }

    /// The new person typed in the editor kept in the store: the name,
    /// the mobile as E.164 (formatted for the region when shown), the
    /// street.
    fn keep_new(&mut self, cx: &Ctx) -> bool {
        if self.first_n == 0 {
            return false;
        }
        let home = phone::plan(*b"US").unwrap_or(&phone::PLANS[0]);
        let mobile = phone::e164(&self.mobile[..self.mobile_n], home);
        let mobile: &[u8] = if self.mobile_n == 0 {
            &[]
        } else {
            mobile.get()
        };
        let mut address = [0u8; 97];
        let mut an = self.street_n;
        address[..an].copy_from_slice(&self.street[..self.street_n]);
        if self.city_n > 0 {
            address[an] = b'\n';
            address[an + 1..an + 1 + self.city_n].copy_from_slice(&self.city[..self.city_n]);
            an += 1 + self.city_n;
        }
        let fields: [(u8, &[u8]); 3] = [
            (tag::FIRST, &self.first[..self.first_n]),
            (tag::MOBILE, mobile),
            (tag::ADDRESS, &address[..an]),
        ];
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let ok = store.put(Kind::Contact, None, &fields).is_ok();
        if ok {
            self.seen = None;
            self.first_n = 0;
            self.mobile_n = 0;
            self.street_n = 0;
        }
        ok
    }

    /// Shows `view`.
    pub fn show(&mut self, view: View) {
        self.view = view;
    }

    /// Types into the editor's fields (the previews).
    pub fn typed(&mut self, first: &[u8], mobile: &[u8], street: &[u8]) {
        let put = |buf: &mut [u8], n: &mut usize, s: &[u8]| {
            let k = s.len().min(buf.len());
            buf[..k].copy_from_slice(&s[..k]);
            *n = k;
        };
        put(&mut self.first, &mut self.first_n, first);
        put(&mut self.mobile, &mut self.mobile_n, mobile);
        put(&mut self.street, &mut self.street_n, street);
    }

    /// Suggestions read from an address service's answer (Photon's
    /// GeoJSON): the network's answer, and the previews' canned one.
    pub fn set_places_from(&mut self, answer: &[u8]) {
        let mut found = [rp_pim::net::Place {
            street: rp_pim::net::Short::default(),
            city: rp_pim::net::Short::default(),
        }; 5];
        let n = rp_pim::net::photon(answer, &mut found);
        self.places = [None; 5];
        for (slot, p) in self.places.iter_mut().zip(&found[..n]) {
            *slot = Some(Place {
                street: Text::of(p.street.get()),
                city: Text::of(p.city.get()),
            });
        }
        self.offline = false;
    }

    /// Offline: the addresses already kept whose street starts as typed.
    fn places_kept(&mut self, cx: &Ctx) {
        self.places = [None; 5];
        self.offline = true;
        let typed = &self.street[..self.street_n];
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        let mut k = 0;
        for r in store.all(Kind::Contact) {
            let a = r.text(tag::ADDRESS);
            if k < 5 && a.len() >= typed.len() && a[..typed.len()].eq_ignore_ascii_case(typed) {
                let (st, ci) = match a.iter().position(|&c| c == b'\n') {
                    Some(i) => (&a[..i], &a[i + 1..]),
                    None => (a, &b""[..]),
                };
                self.places[k] = Some(Place {
                    street: Text::of(st),
                    city: Text::of(ci),
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
        let st = p.street.get();
        self.street[..st.len()].copy_from_slice(st);
        self.street_n = st.len();
        let ci = p.city.get();
        self.city[..ci.len()].copy_from_slice(ci);
        self.city_n = ci.len();
        self.places = [None; 5];
        self.asked = Text::of(st);
    }

    /// The first names kept that start as the one typed (the strip).
    fn names_like(&mut self, cx: &Ctx) {
        self.names = [Text::EMPTY; 3];
        let typed = &self.first[..self.first_n];
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
                *d = kit::disc(p, 40, kit::IDENTITY[k], None);
            }
            self.big = kit::disc(p, 96, tint::CONTACTS, None);
        }
        self.painted = true;
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
        let q = &self.query[..self.query_n];
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
        let mut y = kit::large_title(f, cx, 36, b"Contacts", &s[..k], tint::CONTACTS);
        // Search.
        let field = SEARCH;
        kit::panel(f, cx, field);
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
                b"Search name, number, city",
            );
        } else {
            kit::text(f, field.x + 12, field.y + 25, cx.fonts.ui, TEXT, q);
        }
        y += 46;
        let searching = !q.is_empty() || self.jump.is_some();
        // The favourites grid: large discs with initials, three across
        // (not while searching: the matches come first).
        if !searching {
            kit::section(f, cx, y, b"FAVOURITES", tint::CONTACTS);
            y += 22;
            let mut col = 0u16;
            for &i in &idx[..n] {
                let Some(p) = self.people[usize::from(i)] else {
                    continue;
                };
                if !p.favourite || col == 4 {
                    continue;
                }
                let x = MARGIN + 6 + col * 108;
                let mut nm = [0u8; 64];
                let nn = name(&p, &mut nm);
                kit::avatar(
                    f,
                    self.disc_for(&nm[..nn]),
                    x + 24,
                    y,
                    40,
                    cx.fonts.ui,
                    &nm[..nn],
                );
                kit::centred(
                    f,
                    Rect::new(x - 4, y, 96, 10),
                    y + 60,
                    cx.fonts.ui,
                    TEXT,
                    p.first.get(),
                );
                col += 1;
            }
            y += 72;
        }
        // Everyone, by letter.
        let mut letter = 0u8;
        for &i in &idx[..n] {
            let Some(p) = self.people[usize::from(i)] else {
                continue;
            };
            let l = p
                .last
                .get()
                .first()
                .copied()
                .unwrap_or(b'#')
                .to_ascii_uppercase();
            if l != letter {
                if y + 24 + 48 > 596 {
                    break;
                }
                letter = l;
                kit::section(f, cx, y, &[l], DIM);
                y += 22;
            }
            if y + 48 > 596 {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE - 24, 46);
            kit::panel(f, cx, r);
            if self.n_rows < self.rows_at.len() {
                self.rows_at[self.n_rows] = (r, i);
                self.n_rows += 1;
            }
            let mut nm = [0u8; 64];
            let nn = name(&p, &mut nm);
            kit::avatar(
                f,
                self.disc_for(&nm[..nn]),
                r.x + 6,
                r.y + 3,
                40,
                cx.fonts.ui,
                &nm[..nn],
            );
            kit::fitted(f, r.x + 56, r.y + 21, cx.fonts.ui, TEXT, &nm[..nn], 240);
            // The match in the name, in the system accent.
            if let Some(at) = holds(&nm[..nn], q) {
                let x = r.x + 56 + cx.fonts.ui.width(&nm[..at]);
                kit::text(
                    f,
                    x,
                    r.y + 21,
                    cx.fonts.ui,
                    kit::FOCUS,
                    &nm[at..at + q.len()],
                );
            }
            let sub = if p.company.is_empty() {
                p.email.get()
            } else {
                p.company.get()
            };
            kit::fitted(f, r.x + 56, r.y + 41, cx.fonts.ui, DIM, sub, 240);
            if !p.phones[0].1.is_empty() {
                let ph = shown(p.phones[0].1.get());
                kit::right(f, r.right() - 10, r.y + 21, cx.fonts.ui, DIM, ph.get());
            }
            y += 50;
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

    fn card(&self, f: &mut Frame, cx: &Ctx, i: u8) {
        let Some(p) = self.people.get(usize::from(i)).copied().flatten() else {
            return;
        };
        let mut nm = [0u8; 64];
        let nn = name(&p, &mut nm);
        // The band: the big avatar on the person's colour.
        let band = Rect::new(0, 36, W, 200);
        kit::panel(f, cx, Rect::new(8, band.y + 4, W - 16, band.h - 8));
        kit::avatar(f, self.big, (W - 96) / 2, 48, 96, cx.fonts.title, &nm[..nn]);
        kit::centred(
            f,
            Rect::new(0, 0, W, 1),
            180,
            cx.fonts.title,
            TEXT,
            &nm[..nn],
        );
        kit::centred(
            f,
            Rect::new(0, 0, W, 1),
            206,
            cx.fonts.ui,
            DIM,
            p.company.get(),
        );
        // The action row.
        let mut y = 246;
        for (k, (word, mark)) in [
            (&b"E-mail"[..], &b"@"[..]),
            (b"Map", b"o"),
            (b"Copy", b"="),
            (b"To PC", b">"),
        ]
        .iter()
        .enumerate()
        {
            let r = Rect::new(MARGIN + k as u16 * 112, y, 104, 64);
            // Words, not stand-in marks, until the Aero icon set draws
            // these (icon-brief.md).
            let _ = mark;
            kit::key(f, cx, r, kit::KeyState::Rest);
            kit::centred(f, r, r.y + 40, cx.fonts.ui, TEXT, word);
        }
        y += 76;
        // Details, grouped.
        for (label, number) in &p.phones {
            let number = number.get();
            if number.is_empty() {
                continue;
            }
            let r = Rect::new(MARGIN, y, WIDE, 52);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 14, r.y + 20, cx.fonts.label, DIM, label);
            kit::text(
                f,
                r.x + 14,
                r.y + 44,
                cx.fonts.ui,
                TEXT,
                shown(number).get(),
            );
            y += 56;
        }
        if !p.email.is_empty() {
            let r = Rect::new(MARGIN, y, WIDE, 52);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 14, r.y + 20, cx.fonts.label, DIM, b"E-MAIL");
            kit::text(f, r.x + 14, r.y + 44, cx.fonts.ui, TEXT, p.email.get());
            y += 56;
        }
        if !p.address[0].is_empty() && y + 74 < 596 {
            let r = Rect::new(MARGIN, y, WIDE, 74);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 14, r.y + 20, cx.fonts.label, DIM, b"HOME");
            kit::text(f, r.x + 14, r.y + 44, cx.fonts.ui, TEXT, p.address[0].get());
            kit::text(f, r.x + 14, r.y + 66, cx.fonts.ui, TEXT, p.address[1].get());
            kit::right(
                f,
                r.right() - 14,
                r.y + 44,
                cx.fonts.label,
                tint::CONTACTS,
                b"MAP",
            );
        }
    }

    fn edit(&self, f: &mut Frame, cx: &Ctx, at: EditField) {
        let mut y = kit::large_title(
            f,
            cx,
            36,
            b"New contact",
            b"Typed fields format themselves",
            tint::CONTACTS,
        );
        let ini = &self.first[..self.first_n];
        let disc = self.disc_for(ini);
        kit::avatar(f, disc, W - MARGIN - 52, 44, 40, cx.fonts.ui, ini);
        let field = |f: &mut Frame, y: u16, label: &[u8], value: &[u8], on: bool, hint: &[u8]| {
            let r = Rect::new(MARGIN, y, WIDE, 44);
            kit::panel(f, cx, r);
            if on {
                f.push(rp_app::Cmd::Outline {
                    r,
                    color: kit::FOCUS,
                });
            }
            kit::text(f, r.x + 12, r.y + 28, cx.fonts.label, DIM, label);
            let x = r.x + 110;
            if value.is_empty() {
                kit::text(f, x, r.y + 29, cx.fonts.ui, kit::alpha(DIM, 0x90), hint);
            } else {
                kit::text(f, x, r.y + 29, cx.fonts.ui, TEXT, value);
            }
            if on {
                let cw = cx.fonts.ui.width(value);
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(x + cw + 1, r.y + 10, 2, 24),
                    color: kit::FOCUS,
                });
            }
        };
        y += 6;
        let home = phone::plan(*b"US").unwrap_or(&phone::PLANS[0]);
        let m = phone::format(&self.mobile[..self.mobile_n], home);
        if at == EditField::Street {
            // The fields above fold into one line, so the street and its
            // suggestions sit above the keyboard.
            let r = Rect::new(MARGIN, y, WIDE, 36);
            kit::panel(f, cx, r);
            kit::fitted(f, r.x + 12, r.y + 25, cx.fonts.ui, DIM, ini, 160);
            kit::right(f, r.right() - 12, r.y + 25, cx.fonts.ui, DIM, m.get());
            y += 40;
        } else {
            field(f, y, b"FIRST", ini, at == EditField::First, b"First name");
            y += 48;
            field(f, y, b"LAST", b"", false, b"Last name");
            y += 48;
            field(
                f,
                y,
                b"MOBILE",
                m.get(),
                at == EditField::Mobile,
                b"(555) 555-0100",
            );
            y += 48;
        }
        field(
            f,
            y,
            b"STREET",
            &self.street[..self.street_n],
            at == EditField::Street,
            b"Start typing an address",
        );
        y += 48;
        if at == EditField::Street && self.places.iter().any(Option::is_some) {
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
        } else {
            field(
                f,
                y,
                b"CITY",
                &self.city[..self.city_n],
                false,
                b"City, state, ZIP",
            );
        }
    }
}

/// Contacts as it first opens (the program's entry and the PC's model).
#[must_use]
pub fn opened(_today: i32) -> Contacts {
    Contacts::new()
}

impl Default for Contacts {
    fn default() -> Self {
        Self::new()
    }
}

impl Program for Contacts {
    fn title(&self) -> &'static str {
        "Contacts"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        match self.view {
            View::List => ("Back", "New"),
            View::Card(_) => ("Back", "Edit"),
            View::Edit(_) => ("Cancel", "Done"),
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        self.paint(cx);
        self.refresh(cx);
        kit::backdrop(f);
        match self.view {
            View::List => self.list(f, cx),
            View::Card(i) => self.card(f, cx, i),
            View::Edit(at) => self.edit(f, cx, at),
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        match (e, self.view) {
            // The softkey Done keeps the new person (the keyboard's Done
            // moves to the next field).
            (Event::Key(Key::SoftRight), View::Edit(_)) => {
                if self.keep_new(cx) {
                    self.view = View::List;
                }
                Reply::Redraw
            }
            (Event::Key(Key::SoftLeft), View::List)
                if self.searching || self.query_n > 0 || self.jump.is_some() =>
            {
                self.searching = false;
                self.query_n = 0;
                self.jump = None;
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
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::List) => {
                if self.bubble.take().is_some() {
                    return Reply::Redraw;
                }
                if SEARCH.contains_point(x, y) {
                    self.searching = true;
                    return Reply::Redraw;
                }
                match self.rows_at[..self.n_rows]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                {
                    Some(&(_, i)) => {
                        self.view = View::Card(i);
                        Reply::Redraw
                    }
                    None => Reply::Nothing,
                }
            }
            (Event::Char(c), View::List) if self.searching && self.query_n < self.query.len() => {
                self.query[self.query_n] = c;
                self.query_n += 1;
                Reply::Redraw
            }
            (Event::Delete, View::List) if self.searching => {
                self.query_n = self.query_n.saturating_sub(1);
                Reply::Redraw
            }
            (Event::Done, View::List) if self.searching => {
                self.searching = false;
                Reply::Redraw
            }
            (Event::Key(Key::SoftLeft), _) => {
                self.view = View::List;
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::List) => {
                self.view = View::Edit(EditField::First);
                Reply::Redraw
            }
            (Event::Char(_) | Event::Delete, View::Edit(at)) => {
                let (buf, n): (&mut [u8], &mut usize) = match at {
                    EditField::First => (&mut self.first, &mut self.first_n),
                    EditField::Mobile => (&mut self.mobile, &mut self.mobile_n),
                    EditField::Street => (&mut self.street, &mut self.street_n),
                };
                match e {
                    Event::Char(c) if *n < buf.len() => {
                        buf[*n] = c;
                        *n += 1;
                    }
                    Event::Delete => *n = n.saturating_sub(1),
                    _ => {}
                }
                if at == EditField::First {
                    self.names_like(cx);
                }
                if at == EditField::Street && self.street_n < 3 {
                    self.places = [None; 5];
                }
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Edit(EditField::Street)) => {
                match (0..3u16).find(|&k| suggestion_row(k).contains_point(x, y)) {
                    Some(k) => {
                        self.take_place(usize::from(k));
                        Reply::Redraw
                    }
                    None => Reply::Nothing,
                }
            }
            (Event::Suggestion(k), View::Edit(EditField::Street)) => {
                self.take_place(usize::from(k));
                Reply::Redraw
            }
            (Event::Suggestion(k), View::Edit(EditField::First)) => {
                let name = self.names[usize::from(k).min(2)];
                if !name.is_empty() {
                    let t = name.get();
                    self.first[..t.len()].copy_from_slice(t);
                    self.first_n = t.len();
                    self.names = [Text::EMPTY; 3];
                }
                Reply::Redraw
            }
            (Event::Done, View::Edit(at)) => {
                self.view = View::Edit(match at {
                    EditField::First => EditField::Mobile,
                    EditField::Mobile | EditField::Street => EditField::Street,
                });
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        if self.view == View::List && self.searching {
            return Some(Field::Line);
        }
        match self.view {
            View::Edit(EditField::First) => Some(Field::Name),
            View::Edit(EditField::Mobile) => Some(Field::Phone),
            View::Edit(EditField::Street) => Some(Field::Address),
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
            _ => [&[], &[], &[]],
        }
    }

    /// The street typed, once it has three characters and differs from
    /// the last one asked: Photon is asked over whatever connection there
    /// is; the answer's round trip paces the asking.
    fn net_ask(&mut self) -> Option<rp_app::NetAsk> {
        if self.view != View::Edit(EditField::Street) || self.street_n < 3 {
            return None;
        }
        let typed = &self.street[..self.street_n];
        if self.asked.get() == typed {
            return None;
        }
        self.asked = Text::of(typed);
        Some(rp_app::NetAsk {
            what: b"ADDR",
            arg: Text::of(typed),
        })
    }

    fn net_answer(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
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
}

#[cfg(test)]
mod tests;
