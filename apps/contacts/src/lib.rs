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

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Event, Field, Frame, Key, MARGIN, Pic, Program, Rect, Reply, W, WIDE};
use rp_pim::phone;

/// People a view holds.
pub const MAX: usize = 48;

/// One person as the views show them.
#[derive(Clone, Copy, Debug)]
pub struct Person {
    /// First name.
    pub first: &'static [u8],
    /// Last name.
    pub last: &'static [u8],
    /// Company.
    pub company: &'static [u8],
    /// Phones: a label and the number as kept (E.164).
    pub phones: [(&'static [u8], &'static [u8]); 2],
    /// E-mail.
    pub email: &'static [u8],
    /// The address, as lines.
    pub address: [&'static [u8]; 2],
    /// A favourite.
    pub favourite: bool,
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

/// An address the network service suggested.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    /// The street line.
    pub street: &'static [u8],
    /// City, state and postal code.
    pub city: &'static [u8],
}

/// The Contacts program.
pub struct Contacts {
    people: [Option<Person>; MAX],
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
        .iter()
        .chain(if p.first.is_empty() {
            &b""[..]
        } else {
            &b" "[..]
        })
        .chain(p.last)
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
            view: View::List,
            first: [0; 32],
            first_n: 0,
            mobile: [0; 24],
            mobile_n: 0,
            street: [0; 48],
            street_n: 0,
            places: [None; 5],
            discs: [None; 8],
            big: None,
            painted: false,
        }
    }

    /// Adds a person (false when full).
    pub fn add(&mut self, p: Person) -> bool {
        match self.people.iter_mut().find(|s| s.is_none()) {
            Some(s) => {
                *s = Some(p);
                true
            }
            None => false,
        }
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

    /// The network service's answer for the street typed.
    pub fn set_places(&mut self, places: &[Place]) {
        for (k, s) in self.places.iter_mut().enumerate() {
            *s = places.get(k).copied();
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
        let key = |i: &u8| self.people[usize::from(*i)].map_or(&b""[..], |p| p.last);
        idx[..n].sort_unstable_by(|a, b| key(a).cmp(key(b)));
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

    fn list(&self, f: &mut Frame, cx: &Ctx) {
        let (idx, n) = self.sorted();
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
        let field = Rect::new(MARGIN, y + 4, WIDE - 24, 36);
        kit::panel(f, cx, field);
        kit::text(
            f,
            field.x + 12,
            field.y + 25,
            cx.fonts.ui,
            DIM,
            b"Search name, number, city",
        );
        y += 46;
        // The favourites grid: large discs with initials, three across.
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
                p.first,
            );
            col += 1;
        }
        y += 72;
        // Everyone, by letter.
        let mut letter = 0u8;
        for &i in &idx[..n] {
            let Some(p) = self.people[usize::from(i)] else {
                continue;
            };
            let l = p.last.first().copied().unwrap_or(b'#').to_ascii_uppercase();
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
            let sub = if p.company.is_empty() {
                p.email
            } else {
                p.company
            };
            kit::fitted(f, r.x + 56, r.y + 41, cx.fonts.ui, DIM, sub, 240);
            if !p.phones[0].1.is_empty() {
                let ph = shown(p.phones[0].1);
                kit::right(f, r.right() - 10, r.y + 21, cx.fonts.ui, DIM, ph.get());
            }
            y += 50;
        }
        // The letter strip.
        let strip = Rect::new(W - MARGIN - 18, 120, 18, 470);
        for (k, c) in (b'A'..=b'Z').enumerate() {
            kit::centred(
                f,
                Rect::new(strip.x, 0, 18, 1),
                strip.y + 12 + k as u16 * 18,
                cx.fonts.label,
                DIM,
                &[c],
            );
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
        f.push(rp_app::Cmd::HGradient {
            r: band,
            left: kit::alpha(kit::colour_of(&nm[..nn]), 0x90),
            right: kit::alpha(kit::colour_of(&nm[..nn]), 0x30),
        });
        kit::avatar(f, self.big, (W - 96) / 2, 48, 96, cx.fonts.title, &nm[..nn]);
        kit::centred(
            f,
            Rect::new(0, 0, W, 1),
            180,
            cx.fonts.title,
            TEXT,
            &nm[..nn],
        );
        kit::centred(f, Rect::new(0, 0, W, 1), 206, cx.fonts.ui, DIM, p.company);
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
            kit::tinted(f, cx, r, tint::CONTACTS, 0x50);
            kit::centred(f, r, r.y + 30, cx.fonts.title, TEXT, mark);
            kit::centred(f, r, r.y + 56, cx.fonts.label, TEXT, word);
        }
        y += 76;
        // Details, grouped.
        for (label, number) in p.phones {
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
            kit::text(f, r.x + 14, r.y + 44, cx.fonts.ui, TEXT, p.email);
            y += 56;
        }
        if !p.address[0].is_empty() && y + 74 < 596 {
            let r = Rect::new(MARGIN, y, WIDE, 74);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 14, r.y + 20, cx.fonts.label, DIM, b"HOME");
            kit::text(f, r.x + 14, r.y + 44, cx.fonts.ui, TEXT, p.address[0]);
            kit::text(f, r.x + 14, r.y + 66, cx.fonts.ui, TEXT, p.address[1]);
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
                kit::text(f, sheet.x + 14, ry + 20, cx.fonts.ui, TEXT, pl.street);
                kit::text(f, sheet.x + 14, ry + 40, cx.fonts.ui, DIM, pl.city);
                if k > 0 {
                    f.push(rp_app::Cmd::Fill {
                        r: Rect::new(sheet.x + 12, ry, sheet.w - 24, 1),
                        color: kit::alpha(TEXT, 0x30),
                    });
                }
            }
        } else {
            field(f, y, b"CITY", b"", false, b"City");
        }
    }
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
        kit::backdrop(f);
        match self.view {
            View::List => self.list(f, cx),
            View::Card(i) => self.card(f, cx, i),
            View::Edit(at) => self.edit(f, cx, at),
        }
    }

    fn event(&mut self, e: Event, _cx: &Ctx) -> Reply {
        match (e, self.view) {
            (Event::Key(Key::SoftLeft), View::List) => Reply::Close,
            (Event::Key(Key::SoftLeft), _) => {
                self.view = View::List;
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::List) => {
                self.view = View::Edit(EditField::First);
                Reply::Redraw
            }
            (Event::Char(c), View::Edit(at)) => {
                let (buf, n): (&mut [u8], &mut usize) = match at {
                    EditField::First => (&mut self.first, &mut self.first_n),
                    EditField::Mobile => (&mut self.mobile, &mut self.mobile_n),
                    EditField::Street => (&mut self.street, &mut self.street_n),
                };
                if *n < buf.len() {
                    buf[*n] = c;
                    *n += 1;
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
        match self.view {
            View::Edit(EditField::First) => Some(Field::Name),
            View::Edit(EditField::Mobile) => Some(Field::Phone),
            View::Edit(EditField::Street) => Some(Field::Address),
            _ => None,
        }
    }

    fn suggestions(&self) -> [&[u8]; 3] {
        match self.view {
            View::Edit(EditField::First) => [b"Mae", b"Maeve", b"Mark"],
            View::Edit(EditField::Street) => [b"Pine St", b"Pike Pl", b"Pinehurst"],
            _ => [&[], &[], &[]],
        }
    }
}
