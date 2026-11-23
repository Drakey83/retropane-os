//! A person's address on the map (contacts-rule0.md "Where they are": the
//! address on the radar's own map with a pin), opened by the card's Map
//! key.
//!
//! Iterate, don't imitate (rule 0):
//! - **Their way.** Windows Mobile 5 had no map. Phones hand the address
//!   to a maps app: a geocoding service turns the text into a place on
//!   every opening and a tile server streams the streets, nothing kept
//!   unless a region is downloaded by hand.
//! - **Ours.** The place comes from the answer the address suggestions
//!   already fetched (Photon's coordinates ride with each suggestion,
//!   `rp_pim::net::Place::at`): picking a suggestion geocodes the address
//!   with no question of its own, and an address typed by hand is asked
//!   once, when its map is first opened. The place is kept on the record
//!   ([`rp_pim::store::tag::GEO`], eight bytes), so the pin never needs the
//!   network again; the map is the radar's (the shell's `prog_map`: drawn
//!   once by the relay from Natural Earth, kept on the card, coloured in
//!   the look shown).
//! - **Why it wins.** One fetch where phones make two (suggest, then
//!   geocode); a map that opens offline once seen; the same look and data
//!   as the radar; any country on Earth.

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Frame, MARGIN, MapAsk, Pic, Rect};

/// The zooms the map offers (+ and -): 8 is ~200 km across at mid
/// latitudes (the region), 10 ~50 km (the town and its roads: Natural
/// Earth's finest, the relay's last).
pub const ZOOMS: core::ops::RangeInclusive<u8> = 8..=10;
/// The zoom a map opens at.
pub const OPENS_AT: u8 = 10;

/// Where the map picture sits on the page.
pub const MAP_AT: (u16, u16) = (MARGIN, 120);
/// The zoom keys, + over -, glass on the map's right edge (the owner's
/// pick A of 2026-11-23).
pub const ZOOM_KEYS: [Rect; 2] = {
    let x = MAP_AT.0 + rp_app::kit::MAP_PIC.0 - 56;
    [
        Rect::new(x, MAP_AT.1 + 12, 44, 44),
        Rect::new(x, MAP_AT.1 + 62, 44, 44),
    ]
};

/// The address's card under the map (a tap copies it).
pub const ADDRESS_CARD: Rect = Rect::new(
    MAP_AT.0,
    MAP_AT.1 + rp_app::kit::MAP_PIC.1 + 10,
    rp_app::kit::MAP_PIC.0,
    62,
);

/// What the map page knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The place is being found (the address asked once).
    Finding,
    /// The place is known; the map is being drawn or read.
    Drawing,
    /// On screen.
    Shown,
    /// The address was asked and nothing was found.
    NotFound,
    /// No connection to find the place (an address typed by hand, never
    /// opened online).
    Offline,
    /// The place is known but no map can be had yet (not kept, no
    /// connection): the coordinates are shown.
    NoMap,
}

/// The map page's state.
#[derive(Clone, Copy, Debug)]
pub struct MapPage {
    /// The person shown (their record).
    pub id: u32,
    /// Their place, millionths of a degree.
    pub at: Option<(i32, i32)>,
    /// The map's zoom ([`ZOOMS`]).
    pub zoom: u8,
    /// What the page shows.
    pub status: Status,
    /// The map lent by the shell (its zoom: a stale one is not drawn).
    pub pic: Option<(Pic, u8)>,
    /// The address asked about (once).
    pub asked: bool,
    /// The pin, painted once in the person's colour.
    pub pin: Option<(Pic, u32)>,
    /// Ticks the address card's COPIED mark has left.
    pub copied: u8,
}

impl MapPage {
    /// The page for person `id`, their kept place if any.
    #[must_use]
    pub fn new(id: u32, at: Option<(i32, i32)>) -> Self {
        Self {
            id,
            at,
            zoom: OPENS_AT,
            status: if at.is_some() {
                Status::Drawing
            } else {
                Status::Finding
            },
            pic: None,
            asked: false,
            pin: None,
            copied: 0,
        }
    }

    /// The map to ask the shell for.
    #[must_use]
    pub fn ask(&self) -> Option<MapAsk> {
        let (lat, lon) = self.at?;
        Some(MapAsk {
            lat,
            lon,
            zoom: self.zoom,
        })
    }

    /// A zoom step (+1 closer, -1 farther), inside [`ZOOMS`]: whether it
    /// moved.
    pub fn zoom_by(&mut self, by: i8) -> bool {
        let z = self.zoom.saturating_add_signed(by);
        if !ZOOMS.contains(&z) || z == self.zoom {
            return false;
        }
        self.zoom = z;
        if self.status == Status::Shown {
            self.status = Status::Drawing;
        }
        true
    }
}

/// A place as the record keeps it ([`rp_pim::store::tag::GEO`]): latitude
/// then longitude, millionths, little-endian.
#[must_use]
pub fn geo_bytes((lat, lon): (i32, i32)) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&lat.to_le_bytes());
    b[4..].copy_from_slice(&lon.to_le_bytes());
    b
}

/// A kept place read back (`None`: absent, the wrong length, or not a
/// place on Earth: a damaged record is never a pin in the sea).
#[must_use]
pub fn geo_of(b: &[u8]) -> Option<(i32, i32)> {
    let b: &[u8; 8] = b.try_into().ok()?;
    let lat = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let lon = i32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    (lat.abs() <= 90_000_000 && lon.abs() <= 180_000_000).then_some((lat, lon))
}

/// `v` millionths of a degree as "47.6116" with its hemisphere's letter
/// ("N"/"S" or "E"/"W"), four places (~11 m): the place shown when no map
/// can be had.
pub fn degrees_text(v: i32, pos: u8, neg: u8, out: &mut [u8; 16]) -> usize {
    let a = v.unsigned_abs();
    let (whole, frac) = (a / 1_000_000, (a % 1_000_000) / 100);
    let mut n = 0;
    let mut put = |c: u8, n: &mut usize| {
        if *n < out.len() {
            out[*n] = c;
            *n += 1;
        }
    };
    let mut digits = [0u8; 3];
    let mut k = 0;
    let mut w = whole;
    loop {
        digits[k] = b'0' + (w % 10) as u8;
        k += 1;
        w /= 10;
        if w == 0 || k == 3 {
            break;
        }
    }
    for i in (0..k).rev() {
        put(digits[i], &mut n);
    }
    put(b'.', &mut n);
    for d in [1000, 100, 10, 1] {
        put(b'0' + (frac / d % 10) as u8, &mut n);
    }
    put(0xB0, &mut n); // the degree sign (the shell's fonts draw it)
    put(b' ', &mut n);
    put(if v < 0 { neg } else { pos }, &mut n);
    n
}

/// The pin's picture: a drop in the person's colour, a white heart, a dark
/// rim, anti-aliased (4 x 4 samples a pixel), its tip at the bottom middle.
pub const PIN: (u16, u16) = (28, 36);

/// Paints the pin into `px` ([`PIN`]'s size).
pub fn paint_pin(px: &mut [u32], colour: u32) {
    let (w, h) = (i32::from(PIN.0), i32::from(PIN.1));
    // In 1/16 px: the head's centre and radius, the tip.
    let (cx, cy, r) = (w * 8, 13 * 16, 12 * 16);
    let tip = (h - 1) * 16;
    let inside = |x: i32, y: i32| -> (bool, bool, bool) {
        let (dx, dy) = (x - cx, y - cy);
        let d2 = dx * dx + dy * dy;
        let head = d2 <= r * r;
        // The tail: between the head's two tangents to the tip.
        let tail = y > cy && y <= tip && dx.abs() * (tip - cy) <= r * (tip - y) * 9 / 10;
        let heart = d2 <= (r * 2 / 5) * (r * 2 / 5);
        let body = head || tail;
        let rim = body && {
            let rr = r - 24;
            !(d2 <= rr * rr
                || y > cy && y <= tip - 24 && dx.abs() * (tip - cy) <= rr * (tip - 24 - y) * 9 / 10)
        };
        (body, rim, heart)
    };
    for y in 0..h {
        for x in 0..w {
            let (mut b, mut rim, mut heart) = (0u32, 0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let (i, r, ht) = inside(x * 16 + sx * 4 + 2, y * 16 + sy * 4 + 2);
                    b += u32::from(i);
                    rim += u32::from(r);
                    heart += u32::from(ht);
                }
            }
            let c = if heart * 2 >= b && b > 0 {
                0xFFFF_FFFF
            } else if rim * 2 >= b && b > 0 {
                kit::alpha(0xFF10_1418, 0xFF)
            } else {
                colour | 0xFF00_0000
            };
            let a = b * 255 / 16;
            px[(y * w + x) as usize] = if b == 0 {
                0
            } else {
                (a << 24) | (c & 0x00FF_FFFF)
            };
        }
    }
}

/// The page (the owner's pick A of 2026-11-23): the person's name over the
/// map, the zoom keys on the map's right edge, the pin at the place, the
/// data's credit in the corner, the address on a glass card under it (a tap
/// copies it).
pub fn draw(f: &mut Frame, cx: &Ctx, page: &mut MapPage, name: &[u8], street: &[u8], city: &[u8]) {
    let (mw, mh) = rp_app::kit::MAP_PIC;
    kit::large_title(f, cx, 36, name, b"Where they are", tint::CONTACTS);
    let frame = Rect::new(MAP_AT.0 - 4, MAP_AT.1 - 4, mw + 8, mh + 8);
    kit::panel(f, cx, frame);
    let map = Rect::new(MAP_AT.0, MAP_AT.1, mw, mh);
    match page.pic {
        Some((pic, z)) if z == page.zoom && page.status == Status::Shown => {
            f.push(rp_app::Cmd::Pic16 {
                x: map.x,
                y: map.y as i16,
                pic,
            });
            // The pin's tip at the place (the picture's middle).
            if let Some(p) = cx.pics {
                let colour = kit::colour_of(name);
                if page.pin.is_none_or(|(_, c)| c != colour) {
                    page.pin = p.alloc(PIN.0, PIN.1).map(|pin| {
                        p.write(pin, |px| paint_pin(px, colour));
                        (pin, colour)
                    });
                }
            }
            if let Some((pin, _)) = page.pin {
                f.push(rp_app::Cmd::Pic {
                    x: map.x + mw / 2 - PIN.0 / 2,
                    y: (map.y + mh / 2 - PIN.1 + 1) as i16,
                    pic: pin,
                });
            }
            // Natural Earth's credit (public domain, credited anyway) on
            // its own small glass.
            let credit = b"Natural Earth";
            let cw = cx.fonts.label.width(credit) + 12;
            let at = Rect::new(map.right() - cw - 6, map.bottom() - 22, cw, 18);
            f.push(rp_app::Cmd::Fill {
                r: at,
                color: kit::alpha(0xFF00_0000, 0x70),
            });
            kit::text(f, at.x + 6, at.y + 14, cx.fonts.label, 0xFFE8_ECF0, credit);
        }
        _ => {
            let (line1, line2): (&[u8], &[u8]) = match page.status {
                Status::Finding => (b"Finding the address...", b""),
                Status::Drawing | Status::Shown => (b"Drawing the map...", b""),
                Status::NotFound => (
                    b"The address wasn't found on the map",
                    b"Edit it, or pick it from the suggestions",
                ),
                Status::Offline => (
                    b"No connection to find the address",
                    b"Its map shows once Wi-Fi or the dock is there",
                ),
                Status::NoMap => (
                    b"No map kept for this place yet",
                    b"It comes with the next connection",
                ),
            };
            kit::centred(f, map, map.y + mh / 2 - 6, cx.fonts.ui, TEXT, line1);
            kit::centred(f, map, map.y + mh / 2 + 18, cx.fonts.ui, DIM, line2);
            if let (Status::NoMap, Some((lat, lon))) = (page.status, page.at) {
                let mut a = [0u8; 16];
                let mut o = [0u8; 16];
                let an = degrees_text(lat, b'N', b'S', &mut a);
                let on = degrees_text(lon, b'E', b'W', &mut o);
                let mut both = [0u8; 34];
                both[..an].copy_from_slice(&a[..an]);
                both[an..an + 2].copy_from_slice(b", ");
                both[an + 2..an + 2 + on].copy_from_slice(&o[..on]);
                kit::centred(
                    f,
                    map,
                    map.y + mh / 2 + 46,
                    cx.fonts.label,
                    DIM,
                    &both[..an + 2 + on],
                );
            }
        }
    }
    // The zoom keys, each live only when it can move.
    let [plus, minus] = ZOOM_KEYS;
    for (r, word, live) in [
        (plus, &b"+"[..], page.zoom < *ZOOMS.end()),
        (minus, b"-", page.zoom > *ZOOMS.start()),
    ] {
        kit::key(f, cx, r, kit::KeyState::Rest);
        let c = if live { TEXT } else { kit::alpha(DIM, 0x70) };
        kit::centred(f, r, r.y + r.h / 2 + 9, cx.fonts.num, c, word);
    }
    // The address on a card of glass under the map.
    let strip = ADDRESS_CARD;
    kit::panel(f, cx, strip);
    kit::fitted(
        f,
        strip.x + 14,
        strip.y + 26,
        cx.fonts.ui,
        TEXT,
        street,
        mw - 28,
    );
    kit::fitted(
        f,
        strip.x + 14,
        strip.y + 50,
        cx.fonts.ui,
        DIM,
        city,
        mw - 28,
    );
    if page.copied > 0 {
        kit::right(
            f,
            strip.right() - 14,
            strip.y + 26,
            cx.fonts.label,
            tint::CONTACTS,
            b"COPIED",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_place_reads_back_and_damage_is_no_place() {
        for at in [
            (47_611_600, -122_336_500),
            (-33_868_800, 151_209_300),
            (0, 0),
            (90_000_000, -180_000_000),
        ] {
            assert_eq!(geo_of(&geo_bytes(at)), Some(at));
        }
        assert_eq!(geo_of(&[]), None);
        assert_eq!(geo_of(&[1, 2, 3]), None);
        assert_eq!(geo_of(&geo_bytes((90_000_001, 0))), None);
        assert_eq!(geo_of(&geo_bytes((0, -180_000_001))), None);
        assert_eq!(geo_of(&[0; 9]), None);
    }

    #[test]
    fn zoom_stays_in_its_range() {
        let mut p = MapPage::new(1, Some((1, 2)));
        assert_eq!(p.zoom, OPENS_AT);
        assert!(!p.zoom_by(1), "10 is the relay's last");
        assert!(p.zoom_by(-1));
        assert!(p.zoom_by(-1));
        assert!(!p.zoom_by(-1));
        assert_eq!(p.zoom, 8);
        assert_eq!(p.ask().map(|a| a.zoom), Some(8));
        assert_eq!(MapPage::new(1, None).ask(), None);
    }

    #[test]
    fn coordinates_read_as_people_write_them() {
        let t = |v: i32, p: u8, n: u8| {
            let mut o = [0u8; 16];
            let k = degrees_text(v, p, n, &mut o);
            o[..k].to_vec()
        };
        assert_eq!(t(47_611_600, b'N', b'S'), b"47.6116\xB0 N");
        assert_eq!(t(-122_336_550, b'E', b'W'), b"122.3365\xB0 W");
        assert_eq!(t(-33_868_800, b'N', b'S'), b"33.8688\xB0 S");
        assert_eq!(t(0, b'N', b'S'), b"0.0000\xB0 N");
        assert_eq!(t(180_000_000, b'E', b'W'), b"180.0000\xB0 E");
    }

    #[test]
    fn the_pin_is_a_drop_with_its_tip_at_the_bottom_middle() {
        let mut px = [0u32; (PIN.0 as usize) * (PIN.1 as usize)];
        paint_pin(&mut px, 0xFF74_B1A7);
        let at = |x: usize, y: usize| px[y * PIN.0 as usize + x];
        // The heart white, the head in the colour, corners clear.
        assert_eq!(at(14, 13) | 0x00FF_FFFF, 0xFFFF_FFFF);
        assert_eq!(at(14, 4) & 0x00FF_FFFF, 0x74_B1A7);
        assert_eq!(at(0, 0), 0);
        assert_eq!(at(27, 35), 0);
        // The tip: something at the bottom row's middle, nothing at its ends.
        assert!(at(14, 34) >> 24 > 0 || at(13, 34) >> 24 > 0);
        assert_eq!(at(3, 34), 0);
    }
}
