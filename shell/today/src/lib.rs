//! Test image (ADR-0021): the shell's Today screen on the X51v.
//!
//! Drawn by the 2700G's 2D engine (ADR-0023, option B: `rp-mbx-render`):
//! the wallpaper, converted once to XRGB8888 in 2700G memory, is copied
//! under each changed rectangle and `rp-shell`'s frame drawn over it into
//! one of two opaque 32 bpp pictures, then the screen flips to it. The CPU
//! only describes frames. ([`show`], the earlier set-up: wallpaper in the
//! video plane under the CPU renderer's ARGB4444 layer, stays as the
//! reference the GPU path is tested against.) Touch and the D-pad drive
//! the shell; Home goes to Today; the Start menu's "Back to loader"
//! returns.
//!
//! Tickless: between events the CPU sleeps until the pen or a button
//! interrupts, or an alarm (the pen sampled every 5 ms while it is down, a
//! held key's repeat, the next minute for the clock, a notice's end).
//!
//! **The 50 ms promise, measured** (design brief): every pen-down and
//! pen-up that changes the screen is timed from the moment it is read to
//! the end of the flush that shows it; the run log gets the count, median
//! and worst. On the glass it shows at the next refresh after the flush
//! (at most one more frame; the vsync wait isn't known yet, docs/hw/2700g.md).
//!
//! The clock starts from the build machine's local time (no RTC driver
//! yet). Uses 2700G memory from 6 MiB up and the scratch region; every
//! display register it changes is put back before it returns.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod clock;
pub mod fonts;
pub mod replay;

use rp_draw::Image;

/// Printed in the run log once the Today screen is up.
pub const MARKER: &str = "RETROPANE-TODAY-UP";

/// The shell's fonts, as generated.
#[must_use]
pub fn shell_fonts() -> rp_shell::Fonts {
    rp_shell::Fonts {
        ui: &fonts::UI,
        label: &fonts::LABEL,
        big: &fonts::BIG,
    }
}

/// The weather pictures, from `local/icons/icons.rs` (made by
/// `tools/shell-assets/icons.py` from the default pack; packed by the
/// build as one Paeth-filtered zlib stream; without it there are none and
/// the screens draw outlines).
mod icon_data {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

/// How many pictures are baked in.
pub const ICON_COUNT: usize = icon_data::COUNT;
/// Words the unpacked pictures need.
pub const ICON_WORDS: usize = icon_data::WORDS;

/// Undoes the build's 4-byte Paeth filter (each channel against its own
/// left, up and up-left) in place: `px` is `w` x `h` pixels of 4 bytes.
fn unpaeth4(px: &mut [u8], w: usize, h: usize) {
    let stride = w * 4;
    for y in 0..h {
        for x in 0..stride {
            let i = y * stride + x;
            let a = if x >= 4 { px[i - 4] } else { 0 };
            let b = if y > 0 { px[i - stride] } else { 0 };
            let c = if x >= 4 && y > 0 {
                px[i - stride - 4]
            } else {
                0
            };
            px[i] = px[i].wrapping_add(rp_inflate::paeth(a, b, c));
        }
    }
}

/// Unpacks the baked pictures into `px` (at least [`ICON_WORDS`] words; the
/// image's comes from SDRAM outside the image area) with their
/// descriptions in `imgs`, and returns the shell's set. Too little room or
/// a damaged stream gives no pictures, never a fault.
#[must_use]
pub fn icons_into(
    px: &'static mut [u32],
    imgs: &'static mut [Image; ICON_COUNT],
) -> rp_shell::Icons {
    let desc = icon_data::DESC.iter().copied();
    fill(desc, icon_data::PACKED, ICON_WORDS, px, imgs).unwrap_or(rp_shell::Icons::NONE)
}

/// A card's icon pack (one of the seven, each a picture set: every
/// condition at every size, put on the card by `cargo xtask load`):
/// `RPIK`, version 2, the count, two bytes 0, the words unpacked
/// (little-endian), then per picture its kind (0 big, 1 card, 2 row, 3
/// mid), pack id, width and height, and its stream's length (all
/// little-endian), then each picture's own zlib stream (its Paeth-filtered
/// bytes). Version 1 was one stream for the whole pack: 1.3 s to unpack in
/// one go on the X51v, so a pick waited (the owner, 2026-10-31); a stream a
/// picture lets the image unpack a pack a picture at a time in idle moments
/// as its bytes arrive from the card ([`PackLoad`]).
pub const PACK_MAGIC: &[u8; 4] = b"RPIK";
/// The pack format's version.
pub const PACK_VERSION: u8 = 2;
/// The most pictures a pack holds (22 conditions at four sizes).
pub const PACK_MAX: usize = 88;
/// The header's bytes, and each picture's in the table.
const PACK_HEAD: usize = 12;
const PACK_ENTRY: usize = 10;

/// The words a pack's pictures unpack to, from its header (`None` if it
/// isn't a pack of this version).
#[must_use]
pub fn pack_words(file: &[u8]) -> Option<usize> {
    let h = file.get(..PACK_HEAD)?;
    (&h[..4] == PACK_MAGIC && h[4] == PACK_VERSION)
        .then(|| u32::from_le_bytes([h[8], h[9], h[10], h[11]]) as usize)
}

/// A pack being unpacked a picture at a time, as its bytes arrive. Nothing
/// in it is trusted: the table is checked against the room and its own
/// words before anything is written, every stream must inflate to its
/// picture's exact size, ids are clamped.
pub struct PackLoad {
    /// Pictures unpacked so far.
    done: usize,
    /// Where the next picture's stream starts in the file.
    at: usize,
    /// The room left for pictures.
    rest: &'static mut [u32],
    /// The table checked (count, words) once its bytes are in.
    checked: Option<usize>,
}

/// What a [`PackLoad::step`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackStep {
    /// A picture unpacked; more to come.
    Unpacked,
    /// The next picture's bytes haven't arrived yet.
    Waiting,
    /// Every picture unpacked: [`PackLoad::icons`] makes the set.
    Done,
    /// Not a sound pack, or it doesn't fit: the built-in set stays.
    Broken,
}

impl PackLoad {
    /// A pack to unpack into `px` (the RAM pool's room, at least
    /// [`pack_words`] words).
    #[must_use]
    pub const fn new(px: &'static mut [u32]) -> Self {
        Self {
            done: 0,
            at: 0,
            rest: px,
            checked: None,
        }
    }

    /// The pack's table: count, and where the streams start.
    fn table(file: &[u8]) -> Option<(usize, &[u8])> {
        let words = pack_words(file)?;
        let count = usize::from(file[5]);
        if count == 0 || count > PACK_MAX {
            return None;
        }
        let table = file.get(PACK_HEAD..PACK_HEAD + count * PACK_ENTRY)?;
        let sum: usize = table
            .as_chunks::<PACK_ENTRY>()
            .0
            .iter()
            .map(|e| {
                usize::from(u16::from_le_bytes([e[2], e[3]]))
                    * usize::from(u16::from_le_bytes([e[4], e[5]]))
            })
            .sum();
        (sum == words).then_some((count, table))
    }

    /// One more picture from `file` (the pack's bytes read so far, from
    /// its start) into the room, described in `imgs[i]`.
    pub fn step(&mut self, file: &[u8], imgs: &mut [Image; PACK_MAX]) -> PackStep {
        let Some((count, table)) = Self::table(file) else {
            // The header and table not all in yet, or not a sound pack.
            let head_ok = file
                .get(..5)
                .is_none_or(|h| &h[..4] == PACK_MAGIC && h[4] == PACK_VERSION);
            let count = file.get(5).map_or(1, |&c| usize::from(c));
            let in_yet = file.len() >= PACK_HEAD + count * PACK_ENTRY;
            return if head_ok && (1..=PACK_MAX).contains(&count) && !in_yet {
                PackStep::Waiting
            } else {
                PackStep::Broken
            };
        };
        if self.checked.is_none() {
            let words = pack_words(file).unwrap_or(usize::MAX);
            if words > self.rest.len() {
                return PackStep::Broken;
            }
            self.checked = Some(count);
            self.at = PACK_HEAD + count * PACK_ENTRY;
        }
        if self.done >= count {
            return PackStep::Done;
        }
        let e = &table[self.done * PACK_ENTRY..(self.done + 1) * PACK_ENTRY];
        let (w, h) = (
            u16::from_le_bytes([e[2], e[3]]),
            u16::from_le_bytes([e[4], e[5]]),
        );
        let len = u32::from_le_bytes([e[6], e[7], e[8], e[9]]) as usize;
        let Some(stream) = file.get(self.at..self.at + len) else {
            return PackStep::Waiting;
        };
        let n = usize::from(w) * usize::from(h);
        let (mine, more) = core::mem::take(&mut self.rest).split_at_mut(n);
        self.rest = more;
        let bytes = rp_hal_mem::words_as_bytes_mut(mine);
        if rp_inflate::zlib(stream, bytes) != Ok(n * 4) {
            return PackStep::Broken;
        }
        unpaeth4(bytes, usize::from(w), usize::from(h));
        imgs[self.done] = Image { w, h, px: mine };
        self.at += len;
        self.done += 1;
        if self.done == count {
            PackStep::Done
        } else {
            PackStep::Unpacked
        }
    }

    /// The shell's set from a finished pack's pictures (`imgs`, as the
    /// steps filled them) and its table.
    #[must_use]
    pub fn icons(file: &[u8], imgs: &'static [Image; PACK_MAX]) -> Option<rp_shell::Icons> {
        let (_, table) = Self::table(file)?;
        let mut set = rp_shell::Icons::NONE;
        for (img, e) in imgs.iter().zip(table.as_chunks::<PACK_ENTRY>().0) {
            let i = usize::from(e[1].clamp(1, 22) - 1);
            match e[0] {
                0 => set.hero[i] = Some(img),
                1 => set.card[i] = Some(img),
                2 => set.row[i] = Some(img),
                _ => set.mid[i] = Some(img),
            }
        }
        Some(set)
    }
}

/// A whole pack at once (the PC's previews; the image goes a picture at a
/// time through [`PackLoad`]).
#[must_use]
pub fn pack_into(
    file: &[u8],
    px: &'static mut [u32],
    imgs: &'static mut [Image; PACK_MAX],
) -> Option<rp_shell::Icons> {
    let mut load = PackLoad::new(px);
    loop {
        match load.step(file, imgs) {
            PackStep::Unpacked => {}
            PackStep::Done => return PackLoad::icons(file, imgs),
            PackStep::Waiting | PackStep::Broken => return None,
        }
    }
}

/// The stream inflated into `px` (its bytes are the pictures' bytes: B, G,
/// R, A, the ARGB word in memory on the X51v), each picture's filter
/// undone in place, and the shell's set made from `desc`.
fn fill(
    desc: impl Iterator<Item = (u8, u8, u16, u16)> + Clone,
    stream: &[u8],
    words: usize,
    px: &'static mut [u32],
    imgs: &'static mut [Image],
) -> Option<rp_shell::Icons> {
    let mut set = rp_shell::Icons::NONE;
    if px.len() < words || desc.clone().count() > imgs.len() {
        return None;
    }
    let bytes = rp_hal_mem::words_as_bytes_mut(&mut px[..words]);
    if words > 0 && rp_inflate::zlib(stream, bytes) != Ok(words * 4) {
        return None;
    }
    let mut rest = px;
    for (slot, (_, _, w, h)) in imgs.iter_mut().zip(desc.clone()) {
        let (mine, more) = rest.split_at_mut(usize::from(w) * usize::from(h));
        rest = more;
        unpaeth4(
            rp_hal_mem::words_as_bytes_mut(mine),
            usize::from(w),
            usize::from(h),
        );
        *slot = Image { w, h, px: mine };
    }
    let imgs: &'static [Image] = imgs;
    for (img, (kind, id, ..)) in imgs.iter().zip(desc) {
        let i = usize::from(id.clamp(1, 22) - 1);
        match kind {
            0 => set.hero[i] = Some(img),
            1 => set.card[i] = Some(img),
            2 => set.row[i] = Some(img),
            _ => set.mid[i] = Some(img),
        }
    }
    Some(set)
}

/// The radar map's place on the screen (the Weather app's page) and its
/// bytes unpacked (three planes of 5, 6 and 5 bits).
pub const MAP_TOP: u16 = 70;
/// See [`MAP_TOP`].
pub const MAP_H: usize = 462;
/// See [`MAP_TOP`].
pub const MAP_BYTES: usize = W * MAP_H * 3;

/// Unpacks the image's radar map into `dst` ([`MAP_BYTES`]).
///
/// # Errors
/// `packed` damaged, or `dst` the wrong size.
pub fn unpack_map(packed: &[u8], dst: &mut [u8]) -> Result<(), rp_inflate::Error> {
    if rp_inflate::zlib(packed, dst)? != MAP_BYTES || dst.len() != MAP_BYTES {
        return Err(rp_inflate::Error::Bad);
    }
    for plane in dst.chunks_mut(W * MAP_H) {
        rp_inflate::unpaeth(plane, W, MAP_H);
    }
    Ok(())
}

/// Row `y` of the map as XRGB8888: the Night look, or Ink made from it
/// (black water, light land shaded by the Night's relief, its light roads
/// and labels turned dark: no second map in the image).
pub fn map_row(map: &[u8], y: usize, ink: bool, row: &mut [u32; W]) {
    let n = W * MAP_H;
    for (x, p) in row.iter_mut().enumerate() {
        let i = y * W + x;
        let (r5, g6, b5) = (
            u32::from(*map.get(i).unwrap_or(&0)),
            u32::from(*map.get(n + i).unwrap_or(&0)),
            u32::from(*map.get(2 * n + i).unwrap_or(&0)),
        );
        let (r, g, b) = (r5 << 3 | r5 >> 2, g6 << 2 | g6 >> 4, b5 << 3 | b5 >> 2);
        *p = if !ink {
            r << 16 | g << 8 | b
        } else if 4 * b > 7 * r.max(1) {
            // Water (and the coast's line): black.
            0x18_1818
        } else {
            let l = (r * 77 + g * 150 + b * 29) >> 8;
            let v = (250 - 2 * l.saturating_sub(30) as i32).clamp(16, 250) as u32;
            v << 16 | v << 8 | v
        };
    }
}

/// The wallpaper under the Weather app's page: the radar map (`Some(ink)`)
/// or the wallpaper itself again (`None`, the whole of it converted
/// afresh); then the renderer makes its background again.
pub fn show_backdrop<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    wallpaper: &[u8],
    map: Option<(&[u8], bool)>,
    s: &mut WallScratch,
) {
    match map {
        Some((m, ink)) => {
            let mut row = [0u32; W];
            for y in 0..MAP_H {
                map_row(m, y, ink, &mut row);
                r.wallpaper_row(MAP_TOP + y as u16, &row);
            }
        }
        None => load_wallpaper(r, wallpaper, Depth::Rgb565, s),
    }
    r.wallpaper_changed();
}

/// The real radar as the page's background (the radar built for real,
/// 2026-10-31): the map's view (CARTO's tiles around the place, decoded on
/// the device) with a rain frame (RainViewer's) blended over it by its
/// alpha, row by row into the renderer's wallpaper under the page.
pub fn show_radar<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    map: &[u32],
    rain: Option<&[u32]>,
) {
    let mut row = [0u32; W];
    for y in 0..MAP_H {
        let at = y * W;
        for (x, p) in row.iter_mut().enumerate() {
            let m = map.get(at + x).copied().unwrap_or(0);
            *p = match rain.and_then(|f| f.get(at + x)) {
                Some(&c) if c >> 24 != 0 => blend(m, c),
                _ => m,
            } | 0xFF00_0000;
        }
        r.wallpaper_row(MAP_TOP + y as u16, &row);
    }
    r.wallpaper_changed();
}

/// `c` (ARGB, straight alpha) over `m`.
fn blend(m: u32, c: u32) -> u32 {
    let a = c >> 24;
    let ch = |s: u32| {
        let (mv, cv) = ((m >> s) & 0xFF, (c >> s) & 0xFF);
        ((mv * (255 - a) + cv * a + 127) / 255) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Hourly's line pieces drawn into `px` (at least
/// [`rp_shell::weather_app::CURVE_WORDS`]), described in `imgs`: the
/// shell's set; too little room gives none (a stepped line).
#[must_use]
pub fn curve_into(
    px: &'static mut [u32],
    imgs: &'static mut [Image; rp_shell::weather_app::CURVE_SPRITES],
) -> [Option<&'static Image>; rp_shell::weather_app::CURVE_SPRITES] {
    use rp_shell::weather_app::{CURVE_SPRITES, CURVE_W, CURVE_WORDS, curve_h, curve_sprite};
    let mut set = [None; CURVE_SPRITES];
    if px.len() < CURVE_WORDS {
        return set;
    }
    let mut rest = px;
    for (k, slot) in imgs.iter_mut().enumerate() {
        let h = curve_h(k);
        let (mine, more) = rest.split_at_mut(usize::from(CURVE_W) * usize::from(h));
        rest = more;
        curve_sprite(mine, k);
        *slot = Image {
            w: CURVE_W,
            h,
            px: mine,
        };
    }
    let imgs: &'static [Image; CURVE_SPRITES] = imgs;
    for (s, img) in set.iter_mut().zip(imgs.iter()) {
        *s = Some(img);
    }
    set
}

/// The Station screen's dials painted into `px` (at least
/// [`rp_shell::station::DIAL_WORDS`]), described in `imgs`: the shell's set;
/// too little room gives none (the screen without its dials).
pub fn dials_into(
    px: &'static mut [u32],
    imgs: &'static mut [Image; rp_shell::station::DIALS.len()],
) -> rp_shell::station::Dials {
    use rp_shell::station::{DIAL_WORDS, DIALS, Dials, dial_pixel};
    let mut set = Dials::NONE;
    if px.len() < DIAL_WORDS {
        return set;
    }
    let mut rest = px;
    for (slot, &(d, w, h)) in imgs.iter_mut().zip(DIALS.iter()) {
        let (mine, more) = rest.split_at_mut(usize::from(w) * usize::from(h));
        rest = more;
        for (i, p) in mine.iter_mut().enumerate() {
            *p = dial_pixel(d, (i % usize::from(w)) as u16, (i / usize::from(w)) as u16);
        }
        *slot = Image { w, h, px: mine };
    }
    let imgs: &'static [Image; DIALS.len()] = imgs;
    for (s, img) in set.imgs.iter_mut().zip(imgs.iter()) {
        *s = Some(img);
    }
    set
}

/// Pixels [`radar_into`] needs.
pub const RADAR_WORDS: usize = rp_weather::radar::CELL_PX;

/// The radar's rain cells drawn into `px` (at least [`RADAR_WORDS`]), their
/// descriptions in `imgs`: the shell's set. Too little room: none (the
/// radar shows the map alone).
#[must_use]
pub fn radar_into(
    px: &'static mut [u32],
    imgs: &'static mut [Image; 3],
) -> [Option<&'static Image>; 3] {
    if px.len() < RADAR_WORDS {
        return [None; 3];
    }
    let mut rest = px;
    for (k, (slot, &(w, h))) in imgs.iter_mut().zip(&rp_weather::radar::CELLS).enumerate() {
        let (mine, more) = rest.split_at_mut(usize::from(w) * usize::from(h));
        rest = more;
        rp_weather::radar::cell(mine, k);
        *slot = Image { w, h, px: mine };
    }
    let imgs: &'static [Image; 3] = imgs;
    [Some(&imgs[0]), Some(&imgs[1]), Some(&imgs[2])]
}

/// Bytes of the wallpaper's planes (4:2:0, `W` x `H`).
pub const WALLPAPER_BYTES: usize = W * H * 3 / 2;

/// Unpacks the image's packed wallpaper (zlib of the Paeth-filtered Y, U
/// and V planes, made by the build: sheet today-image-room.md) into `dst`
/// ([`WALLPAPER_BYTES`]; the image's is in the RAM pool). The checksum is
/// checked; damage is an error, never a fault.
///
/// # Errors
/// `packed` damaged, or `dst` the wrong size.
pub fn unpack_wallpaper(packed: &[u8], dst: &mut [u8]) -> Result<(), rp_inflate::Error> {
    if rp_inflate::zlib(packed, dst)? != WALLPAPER_BYTES || dst.len() != WALLPAPER_BYTES {
        return Err(rp_inflate::Error::Bad);
    }
    let (y, uv) = dst.split_at_mut(W * H);
    let (u, v) = uv.split_at_mut(W * H / 4);
    rp_inflate::unpaeth(y, W, H);
    rp_inflate::unpaeth(u, W / 2, H / 2);
    rp_inflate::unpaeth(v, W / 2, H / 2);
    Ok(())
}

/// Room for [`icons_into`]'s descriptions.
pub const NO_IMAGE: Image = Image {
    w: 0,
    h: 0,
    px: &[],
};

/// Screen width and height.
pub const W: usize = 480;
/// Screen height.
pub const H: usize = 640;
/// ADR-0023's set-up: `wallpaper` (planar 4:2:0) converted to XRGB8888
/// into the renderer's wallpaper, the engine reset and the first picture
/// shown as an opaque 32 bpp plane. Shared by the image and the emulator
/// check's reference. `scratch` is the caller's (the image's is in `.bss`).
pub fn show_gpu<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    wallpaper: &[u8],
    delay_us: fn(u32),
    scratch: &mut WallScratch,
) {
    load_wallpaper(r, wallpaper, Depth::Rgb565, scratch);
    r.show(delay_us);
}

/// How the wallpaper's rows are made. The renderer keeps the wallpaper as
/// RGB565 (ADR-0023 Q2 L1, 2026-10-21), taking each channel's top bits, so
/// [`Depth::Rgb565`] is the wallpaper; [`Depth::Full`] hands it 8 bits a
/// channel, which it truncates: plain 16 bits, no dither (kept for the
/// side-by-side compare, whose other side was 32-bit before L1; kept by
/// the owner as dithered against plain, quality audit C48).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// 8 bits a channel, as converted from the picture.
    Full,
    /// RGB565, dithered once on the CPU (Floyd-Steinberg), each channel
    /// widened back by repeating its top bits (as the engine reads a
    /// 16-bit wallpaper, measured 2026-10-17): the look the owner approved
    /// side by side on 2026-10-17.
    Rgb565,
}

/// Room for [`load_wallpaper`]'s work: a row, and the error the 16-bit
/// dither carries to the next row (~8 KiB: the image keeps it in `.bss`).
pub struct WallScratch {
    row: [u32; W],
    /// Error per channel for this row and the next, one column of margin
    /// each side.
    err: [[[i16; 3]; W + 2]; 2],
}

impl Default for WallScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl WallScratch {
    /// Zeroed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            row: [0; W],
            err: [[[0; 3]; W + 2]; 2],
        }
    }
}

/// A channel `v` (with error) to `bits`, then widened back to 8 bits by
/// repeating its top bits: (the shown value, the error left).
fn quantise(v: i16, bits: u32) -> (u8, i16) {
    let v = v.clamp(0, 255);
    let max = (1i16 << bits) - 1;
    let q = (v * max + 127) / 255;
    let shown = (q << (8 - bits)) | (q >> (2 * bits - 8));
    (shown as u8, v - shown)
}

/// `wallpaper` (planar 4:2:0) into the renderer's wallpaper at `depth`.
/// The caller then calls `wallpaper_changed` if a frame was drawn before.
pub fn load_wallpaper<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    wallpaper: &[u8],
    depth: Depth,
    s: &mut WallScratch,
) {
    wallpaper_rows(wallpaper, depth, s, |y, row| r.wallpaper_row(y, row));
}

/// `wallpaper` (planar 4:2:0) as XRGB8888 rows at `depth`, each handed to
/// `f` with its line number.
pub fn wallpaper_rows(
    wallpaper: &[u8],
    depth: Depth,
    s: &mut WallScratch,
    mut f: impl FnMut(u16, &[u32]),
) {
    use rp_drv_mbx_display::scanout::yuv_to_rgb;
    let (luma, chroma) = wallpaper.split_at((W * H).min(wallpaper.len()));
    let (u, v) = chroma.split_at((W * H / 4).min(chroma.len()));
    s.err = [[[0; 3]; W + 2]; 2];
    for y in 0..H {
        let (cur, next) = (y % 2, (y + 1) % 2);
        s.err[next] = [[0; 3]; W + 2];
        for x in 0..W {
            let c = (y / 2) * (W / 2) + x / 2;
            let (r8, g8, b8) = yuv_to_rgb(
                luma.get(y * W + x).copied().unwrap_or(16),
                u.get(c).copied().unwrap_or(128),
                v.get(c).copied().unwrap_or(128),
            );
            // The default picture (the owner, 2026-10-22), before the dither.
            let (r8, g8, b8) = if rp_draw::look::ON {
                rp_draw::look::apply(r8, g8, b8)
            } else {
                (r8, g8, b8)
            };
            let px = match depth {
                Depth::Full => [r8, g8, b8],
                Depth::Rgb565 => {
                    let mut out = [0u8; 3];
                    for (k, (v8, bits)) in [(r8, 5), (g8, 6), (b8, 5)].into_iter().enumerate() {
                        let (shown, e) = quantise(i16::from(v8) + s.err[cur][x + 1][k] / 16, bits);
                        out[k] = shown;
                        // Floyd-Steinberg: 7 right, 3 below left, 5 below,
                        // 1 below right (sixteenths, divided on use).
                        s.err[cur][x + 2][k] += e * 7;
                        s.err[next][x][k] += e * 3;
                        s.err[next][x + 1][k] += e * 5;
                        s.err[next][x + 2][k] += e;
                    }
                    out
                }
            };
            s.row[x] =
                0xFF00_0000 | u32::from(px[0]) << 16 | u32::from(px[1]) << 8 | u32::from(px[2]);
        }
        f(y as u16, &s.row);
    }
}

/// The probe's test pixels: every red and blue level, and every green one.
pub const PROBE_PIXELS: usize = 64;

/// Test pixel `i` of the probe, RGB565.
#[must_use]
pub const fn probe_pixel(i: usize) -> u16 {
    let (r, g, b) = ((i & 31) as u16, (i & 63) as u16, (31 - (i & 31)) as u16);
    r << 11 | g << 5 | b
}

/// `p` (RGB565) widened to XRGB8888 two ways: (repeating the top bits,
/// filling with zeros).
#[must_use]
pub fn widened(p: u16) -> (u32, u32) {
    let (r, g, b) = (
        u32::from(p >> 11),
        u32::from((p >> 5) & 63),
        u32::from(p & 31),
    );
    let rep = (r << 3 | r >> 2) << 16 | (g << 2 | g >> 4) << 8 | (b << 3 | b >> 2);
    let zero = (r << 3) << 16 | (g << 2) << 8 | (b << 3);
    (rep, zero)
}

/// What the engine made of the probe's pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Widen {
    /// Widened by repeating the top bits (and not by zeros).
    pub replicated: u32,
    /// Widened by zeros (and not by repeating).
    pub zero_filled: u32,
    /// The same both ways (the low levels).
    pub either: u32,
    /// Neither: (the source pixel, what came out), the first one.
    pub other: u32,
    /// The first pixel that was neither.
    pub first_other: Option<(u16, u32)>,
}

/// Sorts what came out for each probe pixel (`got(i)`, colour bits only).
pub fn classify(mut got: impl FnMut(usize) -> u32) -> Widen {
    let mut w = Widen::default();
    for i in 0..PROBE_PIXELS {
        let p = probe_pixel(i);
        let g = got(i) & 0xFF_FFFF;
        let (rep, zero) = widened(p);
        match (g == rep, g == zero) {
            (true, true) => w.either += 1,
            (true, false) => w.replicated += 1,
            (false, true) => w.zero_filled += 1,
            (false, false) => {
                w.other += 1;
                w.first_other.get_or_insert((p, g));
            }
        }
    }
    w
}

/// Asks the 2D engine how it widens a 16-bit source copied into 32 bits
/// (read-only for the screen: it uses `at`, 2700G memory nothing else
/// holds, and 64 words after it). `None` if the engine didn't finish.
pub fn probe_widen<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    at: usize,
) -> Option<Widen> {
    use rp_drv_blit2d::{Format, ROP_SRCCOPY, blit};
    let (e, mem) = r.parts();
    let mut words = [0u32; PROBE_PIXELS / 2];
    for (k, w) in words.iter_mut().enumerate() {
        *w = u32::from(probe_pixel(2 * k)) | u32::from(probe_pixel(2 * k + 1)) << 16;
    }
    mem.store(at, &words);
    let out = at + 256;
    mem.store(out, &[0x5A5A_5A5A; PROBE_PIXELS]);
    mem.sync();
    let (w, n) = blit(
        (Format::BPP32, out, (PROBE_PIXELS * 4) as i32),
        (Format::RGB565, at, (PROBE_PIXELS * 2) as i32),
        (0, 0),
        (0, 0, PROBE_PIXELS as u16, 1),
        ROP_SRCCOPY,
        0,
        0,
    )?;
    e.submit(&w[..n]).ok()?;
    e.finish_with(20_000, |a| mem.load(a)).ok()?;
    Some(classify(|i| mem.load(out + 4 * i).unwrap_or(0)))
}

/// Latencies kept for the median (the rest only count towards the worst).
pub const KEPT: usize = 256;

/// Touch-to-flush times in microseconds.
pub struct Latency {
    kept: [u32; KEPT],
    n: usize,
    count: u32,
    worst: u32,
}

impl Default for Latency {
    fn default() -> Self {
        Self::new()
    }
}

impl Latency {
    /// None yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kept: [0; KEPT],
            n: 0,
            count: 0,
            worst: 0,
        }
    }

    /// One more.
    pub fn add(&mut self, us: u32) {
        if let Some(slot) = self.kept.get_mut(self.n) {
            *slot = us;
            self.n += 1;
        }
        self.count += 1;
        self.worst = self.worst.max(us);
    }

    /// (count, median of the kept ones, worst); median 0 with none.
    #[must_use]
    pub fn summary(&mut self) -> (u32, u32, u32) {
        let kept = &mut self.kept[..self.n];
        kept.sort_unstable();
        let median = kept.get(kept.len() / 2).copied().unwrap_or(0);
        (self.count, median, self.worst)
    }
}

/// The plane path's page pictures (sheet weather-gpu-movement.md, "The plane
/// path build, as built"): rows drawn into the CPU's layer at a time (a
/// band's draw list stays well inside a frame's 160 commands).
pub const PAGE_BAND: u16 = 96;

/// A draw list's key, folded into `h` (FNV-1a over every command and its
/// text): a page's picture is current while its draw list's key is the
/// one it was drawn from, whatever changed it (data, units, theme, time).
#[must_use]
pub fn frame_key(f: &rp_draw::Frame, mut h: u32) -> u32 {
    use rp_draw::Cmd;
    let mut mix = |v: u32| {
        for b in v.to_le_bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    };
    let rect = |r: rp_draw::Rect| u32::from(r.x) | u32::from(r.y) << 16;
    let size = |r: rp_draw::Rect| u32::from(r.w) | u32::from(r.h) << 16;
    // Where it is drawn (a band's rows), then what.
    for d in f.dirty() {
        mix(rect(*d));
        mix(size(*d));
    }
    for c in f.cmds() {
        match *c {
            Cmd::Fill { r, color } | Cmd::Outline { r, color } => {
                mix(if matches!(c, Cmd::Fill { .. }) { 1 } else { 2 });
                mix(rect(r));
                mix(size(r));
                mix(color);
            }
            Cmd::HGradient { r, left, right } => {
                mix(3);
                mix(rect(r));
                mix(size(r));
                mix(left);
                mix(right);
            }
            Cmd::Image { x, y, img } => {
                mix(4);
                mix(u32::from(x) | u32::from(y) << 16);
                // Pictures are placed once: the same place, the same pixels.
                mix(core::ptr::from_ref(img) as usize as u32);
            }
            Cmd::Text {
                x,
                baseline,
                font,
                color,
                text,
            } => {
                mix(5);
                mix(u32::from(x) | u32::from(baseline) << 16);
                mix(core::ptr::from_ref(font) as usize as u32);
                mix(color);
                for &b in f.text_of(text) {
                    mix(u32::from(b));
                }
            }
            Cmd::Clip { r } => {
                mix(6);
                mix(rect(r));
                mix(size(r));
            }
        }
    }
    h
}

/// The FNV-1a start for [`frame_key`].
pub const KEY_START: u32 = 0x811C_9DC5;

/// Reveal's window on the video plane (place x, y, size w, h, the app's row
/// and column at its top left) from the shell's window `r` and how many
/// rows lower the app is read in it (screen row y shows the app's row
/// y + `shift`, as the preview composes it: the big panel starts where the
/// card was): the left edge down to the 8-pixel grid the plane's start
/// needs at 2 bytes a pixel (the width grown to keep the right edge),
/// sizes even, never reading past the app's last row.
#[must_use]
pub fn reveal_window(r: rp_draw::Rect, shift: u16) -> (u16, u16, u16, u16, u16, u16) {
    let x = r.x & !7;
    let w = ((r.w + (r.x - x)).min(W as u16 - x)) & !1;
    let row = (r.y + shift).min(H as u16 - 2);
    let h = (r.h.min(H as u16 - r.y).min(H as u16 - row)) & !1;
    (x, r.y, w.max(2), h.max(2), row, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pack of these pictures, `(kind, id, w, h, pixels)`, as the build
    /// makes it (each picture Paeth-filtered and its own stream).
    fn v2_pack(pics: &[(u8, u8, u16, u16, Vec<u32>)]) -> Vec<u8> {
        let mut f = PACK_MAGIC.to_vec();
        let words: usize = pics.iter().map(|p| p.4.len()).sum();
        f.extend([PACK_VERSION, pics.len() as u8, 0, 0]);
        f.extend((words as u32).to_le_bytes());
        let streams: Vec<Vec<u8>> = pics
            .iter()
            .map(|(_, _, w, _, px)| {
                let raw: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
                let stride = usize::from(*w) * 4;
                let at = |i: isize| if i < 0 { 0 } else { raw[i as usize] };
                let filtered: Vec<u8> = (0..raw.len())
                    .map(|i| {
                        let (y, x) = (i / stride, i % stride);
                        let a = if x >= 4 { at(i as isize - 4) } else { 0 };
                        let b = if y > 0 { at((i - stride) as isize) } else { 0 };
                        let c = if x >= 4 && y > 0 {
                            at((i - stride - 4) as isize)
                        } else {
                            0
                        };
                        raw[i].wrapping_sub(rp_inflate::paeth(a, b, c))
                    })
                    .collect();
                rp_inflate::pack::zlib(&filtered)
            })
            .collect();
        for ((k, id, w, h, _), st) in pics.iter().zip(&streams) {
            f.extend([*k, *id]);
            f.extend(w.to_le_bytes());
            f.extend(h.to_le_bytes());
            f.extend((st.len() as u32).to_le_bytes());
        }
        for st in streams {
            f.extend(st);
        }
        f
    }

    #[test]
    fn a_pack_unpacks_a_picture_at_a_time_as_its_bytes_arrive() {
        let a: Vec<u32> = (0..12 * 8).map(|i| 0xFF00_0000 | (i * 0x01_0307)).collect();
        let b: Vec<u32> = (0..5 * 3)
            .map(|i| 0x8000_0000 | (i * 0x0010_2030))
            .collect();
        let pack = v2_pack(&[(0, 4, 12, 8, a.clone()), (3, 9, 5, 3, b.clone())]);
        let px: &'static mut [u32] = Box::leak(vec![0; 12 * 8 + 15].into_boxed_slice());
        let imgs: &'static mut [Image; PACK_MAX] = Box::leak(Box::new([NO_IMAGE; PACK_MAX]));
        let mut load = PackLoad::new(px);
        // The bytes arrive a few at a time, as the card's reads give them.
        let mut got = 0;
        let mut steps = Vec::new();
        loop {
            match load.step(&pack[..got], imgs) {
                PackStep::Waiting => got = (got + 7).min(pack.len()),
                PackStep::Done => break,
                s => steps.push(s),
            }
            assert!(steps.len() < 4, "{steps:?}");
        }
        assert_eq!(steps, [PackStep::Unpacked]);
        assert_eq!(imgs[0].px, &a[..]);
        assert_eq!(imgs[1].px, &b[..]);
        let set = PackLoad::icons(&pack, imgs).expect("a set");
        assert_eq!(set.hero[3].map(|i| i.w), Some(12));
        assert_eq!(set.mid[8].map(|i| i.h), Some(3));
    }

    #[test]
    fn a_damaged_or_oversized_pack_is_refused_never_trusted() {
        let a: Vec<u32> = (0..6 * 6).map(|i| 0xFF00_0000 | i).collect();
        let pack = v2_pack(&[(1, 2, 6, 6, a)]);
        let room = || -> &'static mut [u32] { Box::leak(vec![0; 36].into_boxed_slice()) };
        let imgs =
            || -> &'static mut [Image; PACK_MAX] { Box::leak(Box::new([NO_IMAGE; PACK_MAX])) };
        // Too little room.
        let small: &'static mut [u32] = Box::leak(vec![0; 35].into_boxed_slice());
        assert_eq!(PackLoad::new(small).step(&pack, imgs()), PackStep::Broken);
        // Version 1, a wrong magic, a table whose sizes don't add up.
        for (at, v) in [(4, 1u8), (0, b'X'), (14, 7)] {
            let mut bad = pack.clone();
            bad[at] = v;
            assert_eq!(
                PackLoad::new(room()).step(&bad, imgs()),
                PackStep::Broken,
                "byte {at}"
            );
        }
        // A stream that inflates to the wrong size.
        let mut bad = pack.clone();
        let n = bad.len();
        bad[n - 6] ^= 0x55;
        assert_ne!(PackLoad::new(room()).step(&bad, imgs()), PackStep::Done);
        assert!(pack_into(&pack, room(), imgs()).is_some());
    }

    /// The pictures unpacked into memory that lives for the test.
    fn host_icons() -> rp_shell::Icons {
        icons_into(
            Box::leak(vec![0; ICON_WORDS].into_boxed_slice()),
            Box::leak(Box::new([NO_IMAGE; ICON_COUNT])),
        )
    }

    /// The image's radar map unpacks; Night shows it as built, Ink turns
    /// its water black and its land light; damage is refused.
    #[test]
    fn the_radar_map_unpacks_and_ink_is_made_from_night() {
        let packed = include_bytes!(concat!(env!("OUT_DIR"), "/radar.zz"));
        let mut map = vec![0u8; MAP_BYTES];
        assert_eq!(unpack_map(packed, &mut map), Ok(()));
        let (mut night, mut ink) = ([0u32; W], [0u32; W]);
        let (mut water, mut land) = (0, 0);
        for y in (0..MAP_H).step_by(7) {
            map_row(&map, y, false, &mut night);
            map_row(&map, y, true, &mut ink);
            for (n, i) in night.iter().zip(&ink) {
                let (r, b) = ((n >> 16) & 0xFF, n & 0xFF);
                if 4 * b > 7 * r.max(1) {
                    assert_eq!(*i, 0x18_1818);
                    water += 1;
                } else {
                    land += 1;
                }
            }
        }
        assert!(water > 1000 && land > 1000, "both: {water} {land}");
        let mut bad = packed.to_vec();
        let mid = bad.len() / 3;
        bad[mid] ^= 0x40;
        assert!(unpack_map(&bad, &mut map).is_err());
    }

    /// Every notice fits its area in the real font (a wider one is drawn
    /// past what it marks changed and left behind).
    #[test]
    fn every_notice_fits_between_the_softkeys() {
        let ui = shell_fonts().ui;
        for n in rp_shell::NOTICES {
            let w = ui.width(n.as_bytes());
            assert!(w <= rp_shell::NOTICE_W, "{n}: {w} px");
        }
    }

    /// The image's own packed wallpaper unpacks to the planes it was made
    /// from, byte for byte; damage is refused.
    #[test]
    fn the_packed_wallpaper_unpacks_to_its_planes() {
        let packed = include_bytes!(concat!(env!("OUT_DIR"), "/wallpaper.zz"));
        let raw = include_bytes!(concat!(env!("OUT_DIR"), "/wallpaper.yuv"));
        let mut out = vec![0u8; WALLPAPER_BYTES];
        assert_eq!(unpack_wallpaper(packed, &mut out), Ok(()));
        assert!(out == raw[..], "unpacked planes differ");
        let mut bad = packed.to_vec();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x10;
        assert!(unpack_wallpaper(&bad, &mut out).is_err());
        assert!(unpack_wallpaper(packed, &mut out[..1000]).is_err());
    }

    #[test]
    fn the_pictures_filter_undoes_exactly_and_short_room_gives_outlines() {
        // A 5 x 3 picture of 4-byte pixels, filtered as the build does.
        let (w, h) = (5, 3);
        let px: Vec<u8> = (0..w * h * 4).map(|i| (i * 37 % 251) as u8).collect();
        let stride = w * 4;
        let mut f = px.clone();
        for i in 0..px.len() {
            let (x, y) = (i % stride, i / stride);
            let a = if x >= 4 { px[i - 4] } else { 0 };
            let b = if y > 0 { px[i - stride] } else { 0 };
            let c = if x >= 4 && y > 0 {
                px[i - stride - 4]
            } else {
                0
            };
            f[i] = px[i].wrapping_sub(rp_inflate::paeth(a, b, c));
        }
        unpaeth4(&mut f, w, h);
        assert_eq!(f, px);
        // Too little room: outlines, not a fault.
        let small = icons_into(
            Box::leak(vec![0; ICON_WORDS.saturating_sub(1)].into_boxed_slice()),
            Box::leak(Box::new([NO_IMAGE; ICON_COUNT])),
        );
        assert!(ICON_WORDS == 0 || small.hero.iter().all(Option::is_none));
        // Room: every picture, the hero opaque somewhere.
        let full = icons_into(
            Box::leak(vec![0; ICON_WORDS].into_boxed_slice()),
            Box::leak(Box::new([NO_IMAGE; ICON_COUNT])),
        );
        if ICON_WORDS > 0 {
            let hero = full.hero.iter().flatten().next().expect("a big picture");
            assert!(hero.px.iter().any(|p| p >> 24 == 0xFF));
        }
    }

    #[test]
    fn the_probe_tells_repeating_from_zeros() {
        // Repeating the top bits: white stays white; zeros: it doesn't.
        assert_eq!(widened(0xFFFF), (0xFF_FFFF, 0xF8_FCF8));
        assert_eq!(widened(0), (0, 0));
        let rep = classify(|i| widened(probe_pixel(i)).0);
        let zero = classify(|i| widened(probe_pixel(i)).1);
        assert_eq!(rep.replicated + rep.either, PROBE_PIXELS as u32);
        assert_eq!(zero.zero_filled + zero.either, PROBE_PIXELS as u32);
        assert!(
            rep.replicated > 50 && zero.zero_filled > 50,
            "{rep:?} {zero:?}"
        );
        let odd = classify(|_| 0x12_3456);
        assert_eq!(odd.other, PROBE_PIXELS as u32);
        assert_eq!(odd.first_other, Some((probe_pixel(0), 0x12_3456)));
    }

    /// The 16-bit wallpaper holds only colours RGB565 can hold (as the
    /// engine would show them, top bits repeated) and, dithered, keeps
    /// the picture's average: a smooth gradient's 16 x 16 blocks within a
    /// level of the full-depth picture's (a block's error partly leaves it
    /// across its edges, so smaller blocks drift more).
    #[test]
    fn the_16_bit_wallpaper_is_rgb565_and_keeps_the_average() {
        use rp_mbx_render::model::{self, Bus, Mem};
        use rp_mbx_render::{Layout, Mbx2dRenderer, STRIDE16, to565};

        // A slow diagonal gradient in every channel (luma, then U and V).
        let mut wall = vec![0u8; W * H * 3 / 2];
        for y in 0..H {
            for x in 0..W {
                wall[y * W + x] = (40 + (x + y) * 160 / (W + H)) as u8;
            }
        }
        for (i, b) in wall[W * H..].iter_mut().enumerate() {
            *b = (110 + (i % (W / 2)) * 40 / (W / 2)) as u8;
        }
        let read = |depth| {
            let mut px = Vec::with_capacity(W * H);
            wallpaper_rows(&wall, depth, &mut WallScratch::new(), |_, row| {
                px.extend_from_slice(row);
            });
            px
        };
        let full = read(Depth::Full);
        let low = read(Depth::Rgb565);
        // The renderer keeps the 16-bit rows exactly (RGB565, the top bits).
        let mem = model::memory();
        let mut r = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        load_wallpaper(&mut r, &wall, Depth::Rgb565, &mut WallScratch::new());
        let at = Layout::X51V.wallpaper;
        let m = mem.borrow();
        for (i, &p) in low.iter().enumerate() {
            let a = at + (i / W) * STRIDE16 + (i % W) * 2;
            let kept = u16::from_le_bytes([m[a], m[a + 1]]);
            assert_eq!(kept, to565(p), "pixel {i}");
            assert_eq!(
                model::widen(kept),
                p | 0xFF00_0000,
                "pixel {i} widened back"
            );
        }
        for &p in &low {
            let r5 = (p >> 19) & 31;
            let g6 = (p >> 10) & 63;
            let b5 = (p >> 3) & 31;
            let as565 = (r5 << 11 | g6 << 5 | b5) as u16;
            assert_eq!(
                p & 0xFF_FFFF,
                widened(as565).0,
                "{p:08x} isn't a 16-bit colour"
            );
        }
        for by in (0..H).step_by(16) {
            for bx in (0..W).step_by(16) {
                for sh in [16, 8, 0] {
                    let sum = |v: &[u32]| -> i64 {
                        (0..16)
                            .flat_map(|dy| (0..16).map(move |dx| (by + dy) * W + bx + dx))
                            .map(|i| i64::from((v[i] >> sh) & 0xFF))
                            .sum()
                    };
                    let d = (sum(&low) - sum(&full)).abs();
                    assert!(
                        d <= 256,
                        "block ({bx}, {by}) channel {sh}: off by {d} / 256"
                    );
                }
            }
        }
    }

    #[test]
    fn latency_summary() {
        let mut l = Latency::new();
        assert_eq!(l.summary(), (0, 0, 0));
        for us in [30_000, 10_000, 20_000] {
            l.add(us);
        }
        assert_eq!(l.summary(), (3, 20_000, 30_000));
        for _ in 0..KEPT * 2 {
            l.add(1);
        }
        let (n, _, worst) = l.summary();
        assert_eq!((n, worst), (3 + 2 * KEPT as u32, 30_000));
    }

    /// The Today screen through the GPU renderer (on the engine model)
    /// against the CPU renderer's layer over the same wallpaper, frame
    /// after frame through a tap on Start, the menu and back: within 3
    /// per channel (the engine weighs alpha out of 256 and truncates each
    /// blend; the CPU rounds out of 255 and mixes once).
    #[test]
    fn gpu_today_matches_the_cpu_renderer() {
        use rp_draw::{Frame, Renderer};
        use rp_drv_mbx_display::scanout::mix;
        use rp_gfx::layer::{Layer, PlaneFormat};
        use rp_gfx::render::CpuRenderer;
        use rp_mbx_render::model::{self, Bus, Mem};
        use rp_mbx_render::{Layout, Mbx2dRenderer, PICTURE_BYTES, STRIDE, STRIDE16};
        use rp_shell::{Input, Shell};

        // A wallpaper with every channel moving: luma, then U and V.
        let mut wall = vec![0u8; W * H * 3 / 2];
        for (i, b) in wall.iter_mut().enumerate() {
            *b = (16 + (i * 7 + i / W * 3) % 220) as u8;
        }
        // The wallpaper as the renderer keeps it (16 bits, dithered once).
        let mut wall16 = Vec::with_capacity(W * H);
        wallpaper_rows(&wall, Depth::Rgb565, &mut WallScratch::new(), |_, row| {
            wall16.extend_from_slice(row);
        });
        let rgb = |x: usize, y: usize| {
            let p = wall16[y * W + x];
            ((p >> 16) as u8, (p >> 8) as u8, p as u8)
        };
        let mem = model::memory();
        let mut gpu = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        show_gpu(&mut gpu, &wall, |_| {}, &mut WallScratch::new());
        gpu.dither = false;
        let mut work = vec![0u8; PICTURE_BYTES];
        let mut plane = vec![0u8; PICTURE_BYTES];
        let mut shell = Shell::new(shell_fonts(), host_icons(), true);
        // The emulator's run showed this; frames differ by their text.
        shell.set_time(rp_shell::Time {
            hour: 12,
            minute: 43,
            weekday: 2,
            day: 13,
            month: 10,
        });
        let mut f = Frame::new();
        let s = rp_shell::START;
        let (sx, sy) = (s.x + s.w / 2, s.y + s.h / 2);
        let mut steps = vec![
            None,
            Some(Input::PenDown { x: sx, y: sy }),
            Some(Input::PenUp { x: sx, y: sy }),
            Some(Input::Key(rp_shell::Key::Down)),
            Some(Input::Key(rp_shell::Key::Home)),
        ];
        // The menu opened and closed again and again: from the third time
        // its areas come from the renderer's cache, and must still match.
        for _ in 0..3 {
            for _ in 0..2 {
                steps.push(Some(Input::PenDown { x: sx, y: sy }));
                steps.push(Some(Input::PenUp { x: sx, y: sy }));
            }
        }
        let mut hits = 0;
        for (k, step) in steps.into_iter().enumerate() {
            if let Some(e) = step {
                shell.input(e);
            }
            shell.frame(&mut f);
            gpu.draw(&f);
            assert!(gpu.poll());
            hits += gpu.last.cache_hits;
            let layer = Layer::in_bytes(&mut work, W as u32, H as u32).unwrap();
            CpuRenderer::new(layer, &mut plane, STRIDE, PlaneFormat::Argb8888).draw(&f);
            let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
            let base = ((scadr & 0x7F_FFFF) * 16) as usize;
            let m = mem.borrow();
            let (mut worst, mut over2) = (0u8, 0usize);
            for y in 0..H {
                for x in 0..W {
                    let at = (y * W + x) * 4;
                    let l = u32::from_le_bytes(plane[at..at + 4].try_into().unwrap());
                    let s = base + y * STRIDE16 + x * 2;
                    let g = model::widen(u16::from_le_bytes([m[s], m[s + 1]]));
                    let (wr, wg, wb) = rgb(x, y);
                    let a = (l >> 24) as u8;
                    for (sh, w, bits) in [(16, wr, 5), (8, wg, 6), (0, wb, 5)] {
                        // The screen is the composition truncated: how far
                        // the reference lies outside the truncated step.
                        let want = mix((l >> sh) as u8, w, a);
                        let lost = 8 - bits;
                        let lo = ((g >> sh) as u8 >> lost) << lost;
                        let hi = lo | ((1u8 << lost) - 1);
                        let d = if want < lo {
                            lo - want
                        } else {
                            want.saturating_sub(hi)
                        };
                        worst = worst.max(d);
                        over2 += usize::from(d > 2);
                    }
                }
            }
            // Measured at 32 bits: 3 at worst, in about a dozen of 921,600
            // channel values a frame, where glass is stacked on glass (each
            // engine blend truncates; the CPU composes the layer once).
            // With the weather icons baked in (local builds; CI builds
            // without them), 114 or 152 a frame, still 3 at worst: their
            // see-through edges over the glass are one more truncating blend
            // (2026-10-17). Now the screen is RGB565: the reference must lie
            // in (or within 3 of) the step the screen shows.
            assert!(
                worst <= 3 && over2 < 256,
                "step {k}: worst {worst}, {over2} channels off by more than 2"
            );
        }
        assert_eq!(gpu.stalls, 0);
        assert!(hits >= 4, "cache hits {hits}");
    }

    /// What each frame of opening and closing the Start menu costs the
    /// CPU: atlases built, pictures uploaded, mid-frame waits, words queued.
    #[test]
    fn menu_frames_cost() {
        use rp_draw::{Frame, Renderer};
        use rp_mbx_render::model::{self, Bus, Mem};
        use rp_mbx_render::{Layout, Mbx2dRenderer};
        use rp_shell::{Input, Shell};

        let wall = vec![128u8; W * H * 3 / 2];
        let mem = model::memory();
        let mut gpu = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        show_gpu(&mut gpu, &wall, |_| {}, &mut WallScratch::new());
        let mut shell = Shell::new(shell_fonts(), host_icons(), true);
        let mut f = Frame::new();
        let s = rp_shell::START;
        let (sx, sy) = (s.x + s.w / 2, s.y + s.h / 2);
        let mut steps = vec![("first", None)];
        for _ in 0..3 {
            steps.push(("start down", Some(Input::PenDown { x: sx, y: sy })));
            steps.push(("start up (menu)", Some(Input::PenUp { x: sx, y: sy })));
            steps.push(("start down", Some(Input::PenDown { x: sx, y: sy })));
            steps.push(("start up (close)", Some(Input::PenUp { x: sx, y: sy })));
        }
        for (name, step) in steps {
            if let Some(e) = step {
                shell.input(e);
            }
            if !shell.needs_frame() {
                eprintln!("{name}: no frame");
                continue;
            }
            shell.frame(&mut f);
            gpu.draw(&f);
            assert!(gpu.poll());
            let t = gpu.last;
            eprintln!(
                "{name}: rects {}, forward {}, hits {}, stores {}, bg {}, atlases {}, images {}, waits {}, words {}, blits {}, px copy {} blend {} text {} dither {}",
                t.rects,
                t.forward,
                t.cache_hits,
                t.cache_stores,
                t.background,
                t.atlases,
                t.images,
                t.waits,
                t.words,
                t.blits,
                t.px_copy,
                t.px_blend,
                t.px_text,
                t.px_dither
            );
        }
    }

    #[test]
    fn a_page_key_changes_with_anything_drawn() {
        let mut s = rp_shell::Shell::new(shell_fonts(), host_icons(), true);
        let mut f = rp_draw::Frame::new();
        s.page_band(1, &mut f, 0, PAGE_BAND);
        let a = frame_key(&f, KEY_START);
        s.page_band(1, &mut f, 0, PAGE_BAND);
        assert_eq!(frame_key(&f, KEY_START), a, "the same list, the same key");
        s.page_band(1, &mut f, PAGE_BAND, PAGE_BAND);
        assert_ne!(frame_key(&f, KEY_START), a, "another band");
        let b = frame_key(&f, KEY_START);
        s.page_band(2, &mut f, PAGE_BAND, PAGE_BAND);
        assert_ne!(frame_key(&f, KEY_START), b, "another page");
    }

    #[test]
    fn reveals_window_sits_on_the_planes_grid_and_keeps_its_edges() {
        for (x, w) in [(16u16, 448u16), (13, 454), (0, 480), (7, 466)] {
            let r = rp_draw::Rect::new(x, 60, w, 301);
            let (wx, wy, ww, wh, row, col) = reveal_window(r, 20);
            assert_eq!(wx % 8, 0);
            assert_eq!(col, wx);
            assert!(wx <= x && wx + ww >= x + w - 1 && wx + ww <= 480, "{x} {w}");
            assert_eq!((ww % 2, wh % 2), (0, 0));
            assert_eq!((wy, wh, row), (60, 300, 80));
            // Read lower, never past the app's last row.
            let (_, _, _, wh, row, _) = reveal_window(rp_draw::Rect::new(x, 300, w, 340), 52);
            assert!(row + wh <= 640, "{row} + {wh}");
        }
    }

    #[test]
    fn the_real_fonts_fit_the_shell() {
        // Every frame of the real Today screen fits, menu open included.
        let mut s = rp_shell::Shell::new(shell_fonts(), host_icons(), true);
        let mut f = rp_draw::Frame::new();
        s.frame(&mut f);
        assert!(!f.overflowed());
        s.input(rp_shell::Input::Key(rp_shell::Key::Enter));
        s.input(rp_shell::Input::Key(rp_shell::Key::Enter));
        assert!(s.menu_open());
        s.frame(&mut f);
        assert!(!f.overflowed());
        // The big font has the clock's digits and the degree sign.
        for c in b"0123456789:\xb0" {
            assert!(
                fonts::BIG.glyph(*c).is_some_and(|g| g.w > 0),
                "{}",
                *c as char
            );
        }
    }
}
