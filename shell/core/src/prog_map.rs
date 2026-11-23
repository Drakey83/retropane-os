//! A program's map (contacts-rule0.md "Where they are": the address on the
//! radar's own map with a pin): the radar's map machinery lent to the open
//! program ([`rp_app::MapAsk`], [`rp_app::Program::map_answer`]).
//!
//! Iterate, don't imitate (rule 0):
//! - **Their way.** Windows Mobile had no map; a phone's Contacts hands the
//!   address to a maps app that streams a vendor's tiles (Google's or
//!   Apple's servers, no offline copy unless the user downloads a region),
//!   and OpenStreetMap's own tiles forbid offline keeping (radar-maps.md
//!   "Findings").
//! - **Ours.** The very map the radar shows: the dock's relay draws the
//!   place's layers once from Natural Earth (`RPNET LAYERS`), the device
//!   keeps the file on its card beside the radar's (`RPMAPS`, the same key,
//!   cap and eviction: a contact in a weather place shares its file) and
//!   colours it in the look shown (Paper or Night), so the map opens with
//!   no connection once it has been seen, in our look, from public-domain
//!   data that may be kept. The place itself comes from the address
//!   suggestions' own answer (no geocoding service of its own).
//! - **Why it wins.** No second map stack, no tile server's terms, nothing
//!   fetched twice, and every country on Earth at the same detail
//!   (Natural Earth's): the international rule by construction.
//!
//! The device's loop does the card and the unpacking (it holds the radar's
//! layer buffer); the shell keeps the state, asks the relay and colours the
//! layers into the program's picture.

use rp_weather::mapcard::Key;

use super::Shell;
use super::weather_net::Ask;

/// Where the program's map stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Stage {
    /// None wanted (or the one wanted is shown).
    #[default]
    Idle,
    /// The device is to look for it (its RAM, then its card).
    Look,
    /// On the card: the device reads and unpacks it.
    Card,
    /// Not kept: the relay's, asked or to be asked.
    Dock,
    /// None can be had (said to the program).
    Failed,
}

/// The program's map: what it asked, its key, how far, and the picture lent
/// (its key and the look it was coloured in).
#[derive(Clone, Copy, Default)]
pub(crate) struct ProgMap {
    ask: Option<rp_app::MapAsk>,
    stage: Stage,
    shown: Option<(Key, bool)>,
    pic: Option<rp_draw::pics::Pic>,
}

/// A program's ask as the radar's map key (thousandths of a degree,
/// rounded to the nearest).
#[must_use]
pub fn key_of(a: rp_app::MapAsk) -> Key {
    let k = |v: i32| (v + if v < 0 { -500 } else { 500 }) / 1000;
    Key {
        lat: k(a.lat),
        lon: k(a.lon),
        zoom: a.zoom,
    }
}

/// The radar view's rows and columns the program's picture is cut from
/// (centred: the place at its middle).
const CUT: (usize, usize) = (
    (rp_weather::maplayers::W - rp_app::kit::MAP_PIC.0 as usize) / 2,
    (rp_weather::maplayers::H - rp_app::kit::MAP_PIC.1 as usize) / 2,
);

/// The 2700G 3D core's 4 x 4 dither matrix (`rp-gpubench` run P2 on the
/// X51v, as `rp_drv_mbx_render::DITHER_3D`): a channel goes one step up
/// where the fraction it loses reaches `t` sixteenths of a step.
const DITHER: [[u32; 4]; 4] = [
    [11, 3, 9, 1],
    [7, 15, 5, 13],
    [10, 2, 12, 4],
    [6, 14, 8, 16],
];

/// An opaque pixel at (`x`, `y`) as RGB565, ordered-dithered on the
/// device's own matrix (the map's terrain shading is smooth gradients:
/// truncated, it bands; the screen it lands on is dithered the same way).
#[must_use]
pub fn dither565(c: u32, x: usize, y: usize) -> u16 {
    let t = DITHER[y & 3][x & 3];
    let ch = |v: u32, bits: u32| {
        let step = 1 << (8 - bits);
        let up = u32::from(v & (step - 1) >= (t * step).div_ceil(16));
        ((v >> (8 - bits)) + up).min((1 << bits) - 1)
    };
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    (ch(r, 5) << 11 | ch(g, 6) << 5 | ch(b, 5)) as u16
}

impl Shell {
    /// The open program's map ask now, its key, and whether that needs
    /// the device (a new ask, or the look changed under the one shown).
    fn prog_map_sync(&mut self) {
        let ask = if self.screen == super::Screen::Program {
            self.program
                .try_borrow()
                .ok()
                .and_then(|p| p.as_ref().and_then(|p| p.map_ask()))
                .filter(|a| (3..=10).contains(&a.zoom))
        } else {
            None
        };
        let light = self.light();
        let m = &mut self.pmap;
        if ask != m.ask {
            m.ask = ask;
            m.stage = if ask.is_some() {
                Stage::Look
            } else {
                Stage::Idle
            };
        } else if let (Some(a), Stage::Idle) = (ask, m.stage)
            && m.shown != Some((key_of(a), light))
        {
            // The look changed under the map shown: coloured again.
            m.stage = Stage::Look;
        }
    }

    /// The program's map key while its map is wanted (a step under way for
    /// another key stops).
    #[must_use]
    pub fn prog_map_key(&self) -> Option<Key> {
        self.pmap.ask.map(key_of)
    }

    /// The program's map for the device to find, once per want: in its RAM
    /// (then [`Self::prog_map_layers`] at once) or on its card (then
    /// [`Self::prog_map_card`]). Called every pass of the device's loop.
    pub fn prog_map_wanted(&mut self) -> Option<Key> {
        self.prog_map_sync();
        (self.pmap.stage == Stage::Look)
            .then(|| self.prog_map_key())
            .flatten()
    }

    /// The device's answer for [`Self::prog_map_wanted`]: kept on its card
    /// (it reads and unpacks it, then [`Self::prog_map_layers`]) or not
    /// (the relay is asked).
    pub fn prog_map_card(&mut self, crc: Option<u32>) {
        if self.pmap.stage != Stage::Look {
            return;
        }
        if crc.is_some() {
            self.pmap.stage = Stage::Card;
        } else {
            self.pmap.stage = Stage::Dock;
            self.want(Ask::ProgMap);
        }
    }

    /// Whether the map layers a `LAYERS` answer brings are the program's
    /// (else the radar's): the device unpacks them, then gives them to
    /// [`Self::prog_map_layers`] instead of the radar.
    #[must_use]
    pub fn layers_for_program(&self) -> bool {
        self.asking == Some(Ask::ProgMap) || self.pmap.stage == Stage::Card
    }

    /// The key of the map a `LAYERS` answer is for: the program's while it
    /// asked, else the radar's place.
    #[must_use]
    pub fn layers_key(&self) -> Key {
        match self.prog_map_key() {
            Some(k) if self.layers_for_program() => k,
            _ => self.radar_key(),
        }
    }

    /// The `LAYERS` question's words for the program's map (after
    /// `RPNET `), as the radar's: place and zoom, checked again by the
    /// relay.
    pub(crate) fn prog_map_line(&self) -> Option<rp_weather::Line> {
        let k = self.prog_map_key()?;
        Some(
            rp_weather::Line::new()
                .push(b"LAYERS ")
                .push(super::weather_net::degrees(k.lat).as_bytes())
                .push(b" ")
                .push(super::weather_net::degrees(k.lon).as_bytes())
                .push(b" ")
                .num(i32::from(k.zoom)),
        )
    }

    /// The device's unpacked layers (`rp_weather::maplayers`, the radar
    /// view's 480 x 462) are the program's map `key`: its middle coloured
    /// in the look shown into the program's picture (the last one given
    /// back first) and lent to it. A key no longer wanted is dropped.
    pub fn prog_map_layers(&mut self, key: Key, layers: &[u8]) {
        let Some(ask) = self.pmap.ask.filter(|a| key_of(*a) == key) else {
            return;
        };
        let light = self.light();
        let look = if light {
            rp_weather::maplayers::Look::Paper
        } else {
            rp_weather::maplayers::Look::Night
        };
        let pic = self.app_pics.and_then(|pics| {
            if let Some(old) = self.pmap.pic.take() {
                pics.free(old);
            }
            let (w, h) = rp_app::kit::MAP_PIC;
            let pic = self.owning(|| pics.alloc(w / 2, h))?;
            let (w, h) = (usize::from(w), usize::from(h));
            let written = pics.write(pic, |px| {
                let mut row = [0u32; rp_weather::maplayers::W];
                for (y, out) in px.chunks_exact_mut(w / 2).take(h).enumerate() {
                    rp_weather::maplayers::colour_row(layers, CUT.1 + y, look, &mut row);
                    let row = &row[CUT.0..CUT.0 + w];
                    for (i, pair) in out.iter_mut().enumerate() {
                        let px = |k: usize| u32::from(dither565(row[k], k, y));
                        *pair = px(2 * i) | px(2 * i + 1) << 16;
                    }
                }
            });
            if written {
                Some(pic)
            } else {
                pics.free(pic);
                None
            }
        });
        self.pmap.pic = pic;
        self.pmap.stage = if pic.is_some() {
            Stage::Idle
        } else {
            Stage::Failed
        };
        self.pmap.shown = pic.map(|_| (key, light));
        self.prog_map_answer(ask, pic);
    }

    /// The program's map can't be had (not kept, and the relay refused or
    /// there is no connection; or the card's file was damaged and the
    /// relay is gone).
    pub fn prog_map_failed(&mut self) {
        if let Some(ask) = self.pmap.ask
            && matches!(self.pmap.stage, Stage::Look | Stage::Card | Stage::Dock)
        {
            self.pmap.stage = Stage::Failed;
            self.prog_map_answer(ask, None);
        }
    }

    /// The device's card read of the program's map failed (a damaged or
    /// torn file): the relay's instead.
    pub fn prog_map_card_failed(&mut self) {
        if self.pmap.stage == Stage::Card {
            self.pmap.stage = Stage::Dock;
            self.want(Ask::ProgMap);
        }
    }

    /// The map given to the program (or none), and the screen drawn again.
    fn prog_map_answer(&mut self, ask: rp_app::MapAsk, pic: Option<rp_draw::pics::Pic>) {
        let (fonts, look) = (self.program_fonts(), self.program_look());
        let reply = {
            let cx = self.program_ctx(&fonts, &look);
            let mut p = self.program.borrow_mut();
            match p.as_mut() {
                Some(p) => self.owning(|| p.map_answer(ask, pic, &cx)),
                None => rp_app::Reply::Nothing,
            }
        };
        if reply != rp_app::Reply::Nothing {
            self.dirty(super::Rect::new(0, 0, super::W, super::H));
        }
    }

    /// What a program is given with a call (as every other call builds it).
    fn program_ctx<'a>(
        &'a self,
        fonts: &'a rp_app::Fonts,
        look: &'a rp_app::Look,
    ) -> rp_app::Ctx<'a> {
        rp_app::Ctx {
            fonts,
            look,
            now: self.program_now(),
            pics: self.app_pics,
            forecast: &self.forecast[..usize::from(self.forecast_n)],
            // The region's date order and the clock chosen (R383).
            us: self.region.month_first,
            clock12: self.region.clock12,
            utc_offset: self.utc_offset,
            shapes: &self.shapes,
            clip: &self.clip,
            pim: Some(&self.apps.store),
        }
    }

    /// The program closed: its map forgotten (its picture went with its
    /// others).
    pub(crate) fn prog_map_closed(&mut self) {
        self.pmap = ProgMap::default();
    }
}
