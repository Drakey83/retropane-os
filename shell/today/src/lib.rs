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
use rp_draw::pics::{Pic, Pics};
pub use rp_shell::{Size, Wanted};

/// Printed in the run log once the Today screen is up.
pub const MARKER: &str = "RETROPANE-TODAY-UP";

/// The local time the image starts its clock from: the build machine's
/// (there is no RTC driver yet; the image and the PC's checks read the same
/// build's).
#[must_use]
pub fn build_time() -> rp_shell::Time {
    let utc: i64 = env!("RP_TODAY_UTC_MIN").parse().unwrap_or(0);
    let off: i64 = env!("RP_TODAY_OFFSET_MIN").parse().unwrap_or(0);
    clock::time_at(utc + off)
}

/// The shell's fonts, as generated.
#[must_use]
pub fn shell_fonts() -> rp_shell::Fonts {
    rp_shell::Fonts {
        ui: &fonts::UI,
        label: &fonts::LABEL,
        big: &fonts::BIG,
    }
}

/// Undoes the build's 4-byte Paeth filter (each channel against its own
/// left, up and up-left) in place: `px` is `w` x `h` pixels of 4 bytes.
fn unpaeth4(px: &mut [u8], w: usize, h: usize) {
    // The first row and each row's first pixel have no up-left: Paeth is
    // plainly the left, or the up (so the loop over the rest has no edge
    // tests: R178, the Paeth undo was ~1/4 of an icon's making).
    let stride = w * 4;
    if w == 0 || px.len() < stride * h {
        return;
    }
    for x in 4..stride {
        px[x] = px[x].wrapping_add(px[x - 4]);
    }
    for y in 1..h {
        let (done, rest) = px.split_at_mut(y * stride);
        let up = &done[(y - 1) * stride..];
        let row = &mut rest[..stride];
        for x in 0..4 {
            row[x] = row[x].wrapping_add(up[x]);
        }
        for x in 4..stride {
            row[x] = row[x].wrapping_add(rp_inflate::paeth(row[x - 4], up[x], up[x - 4]));
        }
    }
}

/// `img` copied into a new picture in `pics` (`None` when no slot takes it).
pub fn store_copy(img: &Image, pics: &Pics) -> Option<Pic> {
    let pic = pics.alloc(img.w, img.h)?;
    if pics.write(pic, |px| px.copy_from_slice(img.px)) {
        Some(pic)
    } else {
        pics.free(pic);
        None
    }
}

/// What a [`Keeper::step`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kept {
    /// Every wanted picture is in, from the pack shown.
    Idle,
    /// A picture no screen shows was given back.
    Freed(Size, usize),
    /// A wanted picture was made (pack id less one).
    Made(Size, usize),
    /// A wanted picture couldn't be made (the pack lacks it, or no slot):
    /// not tried again until the pack changes.
    Missing(Size, usize),
}

/// Keeps the store holding exactly the pictures the shell's screens show
/// (`Shell::wanted`), from the pack chosen: one picture a step, so the
/// image works it in idle moments (P1, P2: docs/design/principles.md). A
/// picture from the pack shown before stays until its replacement is in
/// (a pick never leaves a gap while a slot is spare).
pub struct Keeper {
    /// Per size and condition: the pack its picture was made from (or
    /// tried, and missing), plus one; 0 nothing.
    from: [[u8; 22]; 4],
}

impl Default for Keeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Keeper {
    /// Nothing kept yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { from: [[0; 22]; 4] }
    }

    /// One step: gives back a picture no screen shows, else makes one
    /// that is wanted from `pack` with `make` (which puts it in `pics`).
    pub fn step(
        &mut self,
        shell: &mut rp_shell::Shell,
        pics: &Pics,
        pack: u8,
        mut make: impl FnMut(Size, usize, &Pics) -> Option<Pic>,
    ) -> Kept {
        let want = shell.wanted();
        let tag = pack.saturating_add(1);
        for (z, size) in Size::ALL.into_iter().enumerate() {
            for (i, from) in self.from[z].iter_mut().enumerate() {
                if !want.has(size, i) && *from != 0 {
                    if let Some(p) = shell.pic(size, i) {
                        pics.free(p);
                        shell.set_pic(size, i, None);
                    }
                    *from = 0;
                    return Kept::Freed(size, i);
                }
            }
        }
        // The Today screen's pictures first (its big one and the forecast
        // cards: the owner, 2026-10-31, a decode at size is ~70 ms on the
        // X51v), then the Weather app's; within each, a condition at a time
        // (the maker decodes its big picture once for all its sizes), the
        // hero's first.
        let first = (0..22).find(|&i| want.has(Size::Hero, i)).unwrap_or(0);
        for sizes in [[Size::Hero, Size::Card], [Size::Mid, Size::Row]] {
            for i in core::iter::once(first).chain((0..22).filter(|&i| i != first)) {
                for size in sizes {
                    let z = Size::ALL.iter().position(|&s| s == size).unwrap_or(0);
                    if !want.has(size, i) || self.from[z][i] == tag {
                        continue;
                    }
                    let old = shell.pic(size, i);
                    let mut new = make(size, i, pics);
                    if new.is_none()
                        && let Some(o) = old
                    {
                        // No spare slot: the old one goes first.
                        pics.free(o);
                        shell.set_pic(size, i, None);
                        new = make(size, i, pics);
                    }
                    if let Some(o) = old
                        && Some(o) != new
                    {
                        pics.free(o);
                    }
                    shell.set_pic(size, i, new);
                    self.from[z][i] = tag;
                    return if new.is_some() {
                        Kept::Made(size, i)
                    } else {
                        Kept::Missing(size, i)
                    };
                }
            }
        }
        Kept::Idle
    }
}

/// An icon pack (put on the card by `cargo xtask load`, `RPIC<n>`):
/// `RPIK`, version 2, the count, two bytes 0, the words its pictures take
/// (little-endian), then per picture its kind (0 big; the earlier packs'
/// 1 card, 2 row, 3 mid are skipped), pack id, width and height, and its
/// stream's length (all little-endian), then each picture's own zlib
/// stream (its Paeth-filtered bytes). Option C (docs/design/weather-
/// rule0.md): a pack holds only its big (250 px) pictures and the image
/// reads one picture's stream by its offset ([`PackIndex`]) and makes the
/// sizes the screens show from it ([`Maker`]). Version 1 was one stream
/// for the whole pack (1.3 s to unpack on the X51v).
pub const PACK_MAGIC: &[u8; 4] = b"RPIK";
/// A card file's part (`<stem>A.PAK`, `B`, ...: the files are put on the
/// card in parts of this size, the last shorter; the image finds a byte's
/// part by its offset). The loader receives a file into its image area
/// less 64 KiB it keeps as scratch (886,784 bytes for the payload, name
/// included; a 900,000-byte part was refused "too big", 2026-10-31).
pub const PACK_PART: usize = 800_000;
/// The pack format's version.
pub const PACK_VERSION: u8 = 2;
/// The most pictures a pack's table holds (22 conditions at four sizes).
pub const PACK_MAX: usize = 88;
/// The header's bytes, and each picture's in the table.
const PACK_HEAD: usize = 12;
const PACK_ENTRY: usize = 10;

/// A pack's big pictures, from its table (option C: a pack keeps only
/// its 250 px pictures; a pack with every size still reads, its other
/// pictures skipped): each condition's size and where its stream is.
/// Nothing in the table is trusted: sizes over the hero's 250 x 176, streams
/// past the file's end or overlapping are refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PackIndex {
    /// Per condition (pack id less one): width, height, the stream's
    /// offset in the file and its length (0: none).
    big: [Big; 22],
    /// The file's length the table names.
    len: u32,
}

/// A big picture in a pack: its size and its stream's place.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Big {
    /// Width.
    pub w: u16,
    /// Height.
    pub h: u16,
    /// The stream's offset in the file.
    pub off: u32,
    /// The stream's length (0: the pack lacks it).
    pub len: u32,
}

impl PackIndex {
    /// Bytes from the file's start the table needs: the header's 12, then
    /// what its count says (`None` if `head` isn't a pack's header).
    #[must_use]
    pub fn head_len(head: &[u8]) -> Option<usize> {
        let h = head.get(..PACK_HEAD)?;
        let count = usize::from(h[5]);
        (&h[..4] == PACK_MAGIC && h[4] == PACK_VERSION && (1..=PACK_MAX).contains(&count))
            .then_some(PACK_HEAD + count * PACK_ENTRY)
    }

    /// The index from the file's first [`Self::head_len`] bytes.
    #[must_use]
    pub fn read(head: &[u8]) -> Option<Self> {
        let n = Self::head_len(head)?;
        let table = head.get(PACK_HEAD..n)?;
        let mut big = [Big::default(); 22];
        let mut at = n as u64;
        let side = Size::Hero.side();
        for e in table.as_chunks::<PACK_ENTRY>().0 {
            let (w, h) = (
                u16::from_le_bytes([e[2], e[3]]),
                u16::from_le_bytes([e[4], e[5]]),
            );
            let len = u32::from_le_bytes([e[6], e[7], e[8], e[9]]);
            if e[0] == 0 {
                let i = usize::from(e[1].checked_sub(1)?);
                if i >= 22 || w == 0 || h == 0 || w > side || h > rp_shell::HERO_PIC_H || len == 0 {
                    return None;
                }
                big[i] = Big {
                    w,
                    h,
                    off: u32::try_from(at).ok()?,
                    len,
                };
            }
            at += u64::from(len);
        }
        Some(Self {
            big,
            len: u32::try_from(at).ok()?,
        })
    }

    /// The big picture for `icon` (pack id less one), if the pack has it.
    #[must_use]
    pub fn big(&self, icon: usize) -> Option<Big> {
        self.big.get(icon).copied().filter(|b| b.len > 0)
    }

    /// The file's length its table names.
    #[must_use]
    pub const fn file_len(&self) -> u32 {
        self.len
    }
}

/// Words a big picture takes at most (the maker's scratch): 250 x 176,
/// the hero's box.
pub const BIG_WORDS: usize = 250 * rp_shell::HERO_PIC_H as usize;

/// Makes a pack's pictures at the sizes the shell shows: a condition's big
/// picture inflated once into the scratch (held for its other sizes), then
/// copied (the big size) or made smaller with the fitted filter
/// (`rp_draw::shrink`) into a store picture.
pub struct Maker<'s> {
    big: &'s mut [u32],
    shrink: &'s mut [i32],
    /// The big picture in `big`: the pack's tag, the condition, its size.
    held: Option<(u8, usize, u16, u16)>,
    /// The last make's time in its parts (inflating, shrinking), in the
    /// clock's units.
    pub last: (u32, u32),
    /// Big pictures inflated so far.
    pub inflates: u32,
    /// What times the parts (microseconds on the device; none on the PC).
    clock: fn() -> u32,
}

impl<'s> Maker<'s> {
    /// The scratch words `shrink` needs at most (from 250 px to the
    /// widest size the shell shows).
    #[must_use]
    pub fn shrink_words() -> usize {
        rp_draw::shrink::ALL
            .iter()
            .map(|k| rp_draw::shrink::scratch_words(250, usize::from(k.side), k))
            .max()
            .unwrap_or(0)
    }

    /// A maker with `big` ([`BIG_WORDS`]) and `shrink`
    /// ([`Self::shrink_words`]) as scratch.
    #[must_use]
    pub const fn new(big: &'s mut [u32], shrink: &'s mut [i32]) -> Self {
        Self {
            big,
            shrink,
            held: None,
            last: (0, 0),
            inflates: 0,
            clock: || 0,
        }
    }

    /// Times its parts ([`Self::last`]) with `clock`.
    #[must_use]
    pub const fn timed(mut self, clock: fn() -> u32) -> Self {
        self.clock = clock;
        self
    }

    /// Forgets the big picture held (its pack's file changed).
    pub fn forget(&mut self) {
        self.held = None;
    }

    /// `icon` (pack id less one) at `size` from the pack tagged `tag`,
    /// whose table is `index`, into `pics`; `stream` gives a big
    /// picture's stream bytes (the file in RAM, or a read from the card).
    /// `None`: the pack lacks it,
    /// its stream is damaged, or no slot takes it.
    pub fn make<'b>(
        &mut self,
        tag: u8,
        index: &PackIndex,
        size: Size,
        icon: usize,
        pics: &Pics,
        stream: impl FnOnce(Big) -> Option<&'b [u8]>,
    ) -> Option<Pic> {
        let clock = self.clock;
        let b = index.big(icon)?;
        let n = usize::from(b.w) * usize::from(b.h);
        let t0 = clock();
        if self.held != Some((tag, icon, b.w, b.h)) {
            self.held = None;
            let s = stream(b)?;
            let out = self.big.get_mut(..n)?;
            if !decode_big(s, b.w, b.h, out) {
                return None;
            }
            self.inflates += 1;
            self.held = Some((tag, icon, b.w, b.h));
        }
        let t1 = clock();
        let src = self.big.get(..n)?;
        let (dw, dh) = if size == Size::Hero {
            (b.w, b.h)
        } else {
            rp_draw::shrink::size(b.w, b.h, size.side())
        };
        let pic = pics.alloc(dw, dh)?;
        let k = rp_draw::shrink::Kernel::for_side(size.side());
        let shrink = &mut *self.shrink;
        let mut ok = false;
        pics.write(pic, |px| {
            ok = if size == Size::Hero {
                px.copy_from_slice(src);
                true
            } else {
                rp_draw::shrink::shrink(
                    src,
                    (usize::from(b.w), usize::from(b.h)),
                    px,
                    (usize::from(dw), usize::from(dh)),
                    k,
                    shrink,
                )
            };
        });
        self.last = (t1.wrapping_sub(t0), clock().wrapping_sub(t1));
        if ok {
            Some(pic)
        } else {
            pics.free(pic);
            None
        }
    }
}

/// A big picture's stream (zlib of its Paeth-filtered bytes) into `out`
/// (`w` x `h` words): false, never a fault, if it doesn't inflate to
/// exactly that.
#[must_use]
pub fn decode_big(stream: &[u8], w: u16, h: u16, out: &mut [u32]) -> bool {
    let n = usize::from(w) * usize::from(h);
    let Some(out) = out.get_mut(..n) else {
        return false;
    };
    let bytes = rp_hal_mem::words_as_bytes_mut(out);
    if rp_inflate::zlib(stream, bytes) != Ok(n * 4) {
        return false;
    }
    unpaeth4(bytes, usize::from(w), usize::from(h));
    true
}

/// Every condition of a pack `file` at every size the shell shows, made
/// into `pics` (the previews' and the tests' whole set; the image keeps
/// only what the screens show, [`Keeper`]): the shell's set. None if the
/// file isn't a pack.
pub fn make_all(file: &[u8], pics: &Pics, maker: &mut Maker) -> rp_shell::Icons {
    let mut icons = rp_shell::Icons::NONE;
    let Some(index) = PackIndex::read(file) else {
        return icons;
    };
    maker.forget();
    for i in 0..22 {
        for size in Size::ALL {
            icons.of_mut(size)[i] = maker.make(0, &index, size, i, pics, |b| {
                file.get(b.off as usize..(b.off as usize).checked_add(b.len as usize)?)
            });
        }
    }
    icons
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
/// 2026-10-31): the map's view (OpenStreetMap's tiles around the place,
/// decoded on the device and kept as sent) in the look the settings name
/// (`ink`: Ink, else Night) with a rain frame (RainViewer's) blended over it
/// by its alpha, row by row into the renderer's wallpaper under the page.
pub fn show_radar<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    map: &[u32],
    rain: Option<&[u32]>,
    ink: bool,
) {
    let mut row = [0u32; W];
    for y in 0..MAP_H {
        let at = y * W;
        for (x, p) in row.iter_mut().enumerate() {
            // The map as OpenStreetMap sent it, in the look the settings
            // name: Ink (black water, light land) or Night (dark land).
            let raw = map.get(at + x).copied().unwrap_or(0);
            // No tile there yet: black, as before (darkened, a zero is
            // white).
            let m = if raw == 0 {
                0
            } else if ink {
                rp_weather::tiles::ink(raw)
            } else {
                rp_weather::tiles::dark(raw)
            };
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

/// Hourly's graph picture kept for the forecast shown (`painted`: the
/// key it was painted for): when the forecast's hours change, the old
/// picture is given back and a new one painted into the store
/// (`Shell::paint_graph`, one slot of `GRAPH_PIC`). True if it changed.
pub fn keep_graph(shell: &mut rp_shell::Shell, pics: &Pics, painted: &mut Option<u32>) -> bool {
    let key = shell.graph_key();
    if *painted == Some(key) && shell.graph().is_some_and(|p| pics.is_live(p)) {
        return false;
    }
    if let Some(old) = shell.graph() {
        pics.free(old);
        shell.set_graph(None);
    }
    let (w, h) = shell.graph_size();
    let pic = pics.alloc(w, h);
    if let Some(p) = pic {
        let s: &rp_shell::Shell = shell;
        pics.write(p, |px| s.paint_graph(px));
    }
    shell.set_graph(pic);
    *painted = Some(key);
    true
}

/// The open chart's line kept for the chart shown (`painted`: the key it
/// was painted for), in a picture of its own (`Shell::paint_chart_pic`,
/// `CHART_PIC`): Hourly's left as it is. True if it changed.
pub fn keep_chart(shell: &mut rp_shell::Shell, pics: &Pics, painted: &mut Option<u32>) -> bool {
    let Some(key) = shell.chart_pic_key() else {
        return false;
    };
    if *painted == Some(key) && shell.chart_pic().is_some_and(|p| pics.is_live(p)) {
        return false;
    }
    if let Some(old) = shell.chart_pic() {
        pics.free(old);
        shell.set_chart_pic(None);
    }
    let (w, h) = rp_shell::chart::CHART_PIC;
    let pic = pics.alloc(w, h);
    if let Some(p) = pic {
        let s: &rp_shell::Shell = shell;
        pics.write(p, |px| s.paint_chart_pic(px));
    }
    shell.set_chart_pic(pic);
    *painted = Some(key);
    true
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

/// An empty picture description (room for one made at start).
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
            Cmd::Pic { x, y, pic } => {
                mix(7);
                mix(u32::from(x) | u32::from(y.cast_unsigned()) << 16);
                // A store picture's pixels are final once drawn: its handle
                // (slot and era) names them.
                mix(pic.key());
                mix(u32::from(pic.w) | u32::from(pic.h) << 16);
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
            Cmd::Panel { r, art } => {
                mix(8);
                mix(rect(r));
                mix(size(r));
                // Its art is fixed: the same art, the same pixels.
                mix(core::ptr::from_ref(art) as usize as u32);
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

    /// The Weather app as Reveal shows it (composed ahead, whole, in 2D)
    /// and as it rests (its chrome on the screen, its Now page's picture on
    /// the layer at its window), each drawn by the CPU renderer at 32 bits
    /// over nothing: they must be the same pixels, or the opening's end
    /// changes the picture (the owner's "glitch at the end", 2026-11-01).
    fn reveal_and_rest(light: bool) -> (std::vec::Vec<u32>, std::vec::Vec<u32>) {
        use rp_draw::Renderer as _;
        use rp_gfx::layer::{Layer, PlaneFormat};
        use rp_gfx::render::CpuRenderer;
        let draw =
            |w: usize,
             h: usize,
             fill: &mut dyn FnMut(&mut rp_draw::Frame, &mut CpuRenderer<'_>)| {
                let mut px = std::vec![0u32; w * h];
                let mut plane = std::vec![0u8; w * 4 * h];
                {
                    let mut cpu = CpuRenderer::new(
                        Layer::new(&mut px, w as u32, h as u32, w).expect("the layer"),
                        &mut plane,
                        w * 4,
                        PlaneFormat::Argb8888,
                    );
                    let mut f = rp_draw::Frame::new();
                    fill(&mut f, &mut cpu);
                }
                plane
                    .chunks(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect::<std::vec::Vec<u32>>()
            };
        let mut shell = rp_shell::Shell::new(shell_fonts(), rp_shell::Icons::NONE, true);
        shell.set_time(rp_shell::Time {
            hour: if light { 12 } else { 22 },
            minute: 0,
            weekday: 0,
            day: 1,
            month: 11,
        });
        shell.use_planes(true);
        let mut f = rp_draw::Frame::new();
        shell.frame(&mut f);
        shell.input(rp_shell::Input::PenDown { x: 240, y: 150 });
        shell.input(rp_shell::Input::PenUp { x: 240, y: 150 });
        shell.settle();
        assert_eq!(shell.screen(), rp_shell::Screen::Weather);
        let ahead = draw(W, H, &mut |f, cpu| {
            shell.weather_preview(f);
            let _ = cpu.draw(f);
        });
        let chrome = draw(W, H, &mut |f, cpu| {
            shell.weather_chrome_preview(f);
            let _ = cpu.draw(f);
        });
        let rows = usize::from(rp_shell::picture_rows(0));
        let page = draw(W, rows, &mut |f, cpu| {
            let mut top = 0;
            while top < rows {
                let n = usize::from(PAGE_BAND).min(rows - top);
                shell.page_band(0, f, top as u16, n as u16);
                let _ = cpu.draw(f);
                top += n;
            }
        });
        let v = shell.page_layer().expect("the Now page on the layer");
        // Over a wallpaper (a grey ramp) as the display mixes planes
        // (straight alpha): the screen's pixels, then the layer's over them.
        let over = |p: u32, d: u32| {
            let a = (p >> 24) as u8;
            let ch = |s: u32| {
                u32::from(rp_drv_mbx_display::scanout::mix(
                    (p >> s) as u8,
                    (d >> s) as u8,
                    a,
                )) << s
            };
            0xFF00_0000 | ch(16) | ch(8) | ch(0)
        };
        let wall = |i: usize| {
            let g = (i % W * 255 / W) as u32;
            0xFF00_0000 | g << 16 | (255 - g) << 8 | (i / W * 255 / H) as u32
        };
        let ahead: std::vec::Vec<u32> = ahead
            .iter()
            .enumerate()
            .map(|(i, &p)| over(p, wall(i)))
            .collect();
        let mut rest: std::vec::Vec<u32> = chrome
            .iter()
            .enumerate()
            .map(|(i, &p)| over(p, wall(i)))
            .collect();
        for y in 0..usize::from(v.win.h) {
            for x in 0..usize::from(v.win.w) {
                let (sx, sy) = (usize::from(v.win.x) + x, usize::from(v.win.y) + y);
                let p = page[(usize::from(v.row) + y) * W + usize::from(v.win.x) + x];
                rest[sy * W + sx] = over(p, rest[sy * W + sx]);
            }
        }
        (ahead, rest)
    }

    /// The Weather pages' pictures stay put while nothing on them changes:
    /// on the Station screen, Hourly's picture was built again every ~1.5 s
    /// (the owner's run of 2026-11-01, 11 times in 22 s, ~190 ms of CPU
    /// each): its key must not move with the clock alone.
    #[test]
    fn a_page_s_key_holds_while_nothing_on_it_changes() {
        let key = |shell: &mut rp_shell::Shell, tab: u8| {
            let mut f = rp_draw::Frame::new();
            let rows = rp_shell::picture_rows(tab);
            let (mut k, mut top) = (KEY_START, 0);
            while top < rows {
                let n = PAGE_BAND.min(rows - top);
                shell.page_band(tab, &mut f, top, n);
                k = frame_key(&f, k);
                top += n;
            }
            k
        };
        let storm: &[u8] = br#"{"current_conditions": {"time": 1761900000,
          "air_temperature": 20.0, "relative_humidity": 61, "wind_avg": 12.9, "wind_gust": 33.8,
          "sea_level_pressure": 1016.4, "pressure_trend": "rising", "uv": 6}}"#;
        let mut shell = rp_shell::Shell::new(shell_fonts(), rp_shell::Icons::NONE, true);
        shell.use_planes(true);
        shell.set_station_from_tempest(storm);
        let mut f = rp_draw::Frame::new();
        shell.frame(&mut f);
        shell.input(rp_shell::Input::PenDown { x: 240, y: 150 });
        shell.input(rp_shell::Input::PenUp { x: 240, y: 150 });
        shell.settle();
        shell.input(rp_shell::Input::PenDown { x: 325, y: 53 });
        shell.input(rp_shell::Input::PenUp { x: 325, y: 53 });
        shell.settle();
        std::println!("screen {:?}", shell.screen());
        let mut now = 10_000u32;
        let first: std::vec::Vec<u32> = (0..3).map(|t| key(&mut shell, t)).collect();
        for _ in 0..20 {
            now += 1_500;
            shell.tick(now);
            shell.frame(&mut f);
            let k: std::vec::Vec<u32> = (0..3).map(|t| key(&mut shell, t)).collect();
            assert_eq!(k, first, "at {now} ms on {:?}", shell.screen());
        }
    }

    /// Where a big picture's making goes, on the PC (R178): the inflate
    /// and the Paeth undo apart, over a real pack (`target/load/
    /// relay-rp-icons0.pak`, the default, made by a load).
    #[test]
    #[ignore = "exploration: times a pack's pictures on the PC"]
    fn where_a_big_picture_s_time_goes() {
        let Ok(file) = std::fs::read("../../target/load/relay-rp-icons0.pak") else {
            std::println!("no pack: run a load first");
            return;
        };
        let idx = PackIndex::read(&file).expect("a pack");
        let (mut tz, mut tp, mut n) = (0u128, 0u128, 0);
        let mut out = std::vec![0u32; 250 * 250];
        for i in 0..22 {
            let Some(b) = idx.big(i).filter(|b| b.len > 0) else {
                continue;
            };
            let stream = &file[b.off as usize..(b.off + b.len) as usize];
            let px = usize::from(b.w) * usize::from(b.h);
            for _ in 0..20 {
                let bytes = rp_hal_mem::words_as_bytes_mut(&mut out[..px]);
                let t0 = std::time::Instant::now();
                let _ = rp_inflate::zlib(stream, bytes);
                let t1 = std::time::Instant::now();
                unpaeth4(bytes, usize::from(b.w), usize::from(b.h));
                let t2 = std::time::Instant::now();
                tz += (t1 - t0).as_nanos();
                tp += (t2 - t1).as_nanos();
                n += 1;
            }
        }
        std::println!(
            "per picture on the PC: inflate {} us, unpaeth {} us",
            tz / n / 1000,
            tp / n / 1000
        );
    }

    #[test]
    fn reveal_ends_on_the_same_picture_the_app_rests_on() {
        for light in [false, true] {
            let (ahead, rest) = reveal_and_rest(light);
            if let Some(dir) = option_env!("RP_REVEAL_DUMP") {
                let raw = |v: &[u32]| {
                    v.iter()
                        .flat_map(|p| p.to_le_bytes())
                        .collect::<std::vec::Vec<u8>>()
                };
                std::fs::write(std::format!("{dir}/ahead-{light}.raw"), raw(&ahead)).unwrap();
                std::fs::write(std::format!("{dir}/rest-{light}.raw"), raw(&rest)).unwrap();
            }
            let mut n = 0;
            let mut b = (W, H, 0, 0);
            for y in 0..H {
                for x in 0..W {
                    let (p, q) = (ahead[y * W + x], rest[y * W + x]);
                    let off =
                        (0..4).any(|k| ((p >> (8 * k)) & 0xFF).abs_diff((q >> (8 * k)) & 0xFF) > 3);
                    if off {
                        n += 1;
                        b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
                    }
                }
            }
            assert_eq!(n, 0, "light {light}: {n} pixels differ, within {b:?}");
        }
    }

    /// The Today screen's text and the living sky fit the 2D engine's glyph
    /// memory together (an atlas a font and colour, as the renderer builds
    /// them): over it, every frame would reset the atlases and upload the
    /// fonts again. Counted from the fonts and colours a storm's Today
    /// frame draws, by day and by night.
    #[test]
    fn the_sky_and_the_screens_fonts_fit_the_glyph_memory() {
        let atlas = |f: &rp_draw::Font| -> usize {
            f.glyphs
                .iter()
                .map(|g| (usize::from(g.w) * usize::from(g.h) * 4).next_multiple_of(16))
                .sum()
        };
        let room = rp_mbx_render::Layout::X51V.glyphs.1;
        for hour in [12, 22] {
            let mut shell = rp_shell::Shell::new(shell_fonts(), rp_shell::Icons::NONE, true);
            shell.set_time(rp_shell::Time {
                hour,
                minute: 0,
                weekday: 0,
                day: 1,
                month: 11,
            });
            let mut wx = *shell.forecast();
            wx.now.sky.wmo = 95;
            wx.now.sky.cover = 100;
            shell.set_forecast(wx);
            shell.preview_sky(true);
            let mut f = rp_draw::Frame::new();
            shell.frame(&mut f);
            let mut seen: std::vec::Vec<(usize, u32)> = std::vec::Vec::new();
            let mut total = 0;
            for c in f.cmds() {
                if let rp_draw::Cmd::Text { font, color, .. } = *c {
                    let key = (core::ptr::from_ref(font) as usize, color);
                    if !seen.contains(&key) {
                        seen.push(key);
                        total += atlas(font);
                    }
                }
            }
            // The renderer's slots (ADR-0023): more, and every frame
            // would reset the cache.
            assert!(seen.len() <= 16, "{} atlases", seen.len());
            assert!(total <= room, "at {hour}:00 {total} of {room} bytes");
        }
    }

    /// Every store picture the 2D engine draws fits its picture cache
    /// whole (a bigger one draws nothing: Hourly's 446 x 160 graph, 285 KB,
    /// was never drawn on the 2D path; ui-preview, 2026-11-01).
    #[test]
    fn every_picture_fits_the_2d_engines_cache() {
        let cache = rp_mbx_render::Layout::X51V.images.1;
        let sizes = [
            (rp_shell::Size::Hero.side(), rp_shell::HERO_PIC_H),
            (rp_shell::Size::Mid.side(), rp_shell::Size::Mid.side()),
            (rp_shell::Size::Card.side(), rp_shell::Size::Card.side()),
            (rp_shell::Size::Row.side(), rp_shell::Size::Row.side()),
            rp_shell::weather_app::GRAPH_PIC,
            rp_shell::chart::CHART_PIC,
        ];
        for (w, h) in sizes {
            assert!(
                usize::from(w) * usize::from(h) * 4 <= cache,
                "{w} x {h} is over the cache's {cache} bytes"
            );
        }
        // The chart is painted into the graph's slot.
        let (g, c) = (rp_shell::weather_app::GRAPH_PIC, rp_shell::chart::CHART_PIC);
        assert!(c.0 <= g.0 && c.1 <= g.1);
    }

    /// A pack of big pictures only (option C) reads into an index; the
    /// maker inflates a condition once for all its sizes, copies the big
    /// one exactly and makes the others at the shell's sizes; a damaged
    /// stream, a missing condition or a table that lies is refused.
    #[test]
    fn the_maker_decodes_a_big_picture_once_for_every_size() {
        let (w, h) = (200u16, 140u16);
        let a: Vec<u32> = (0..u32::from(w) * u32::from(h))
            .map(|i| 0xFF00_0000 | (i % 200) << 16 | (i / 200) << 8 | 0x40)
            .collect();
        let pack = v2_pack(&[(0, 5, w, h, a.clone()), (1, 5, 2, 2, vec![0; 4])]);
        let index = PackIndex::read(&pack).expect("an index");
        assert_eq!(index.file_len() as usize, pack.len());
        assert!(
            index.big(5).is_none() && index.big(4).is_some(),
            "only the big one"
        );
        let pics = host_store();
        let mut big = vec![0; BIG_WORDS];
        let mut sh = vec![0; Maker::shrink_words()];
        let mut m = Maker::new(&mut big, &mut sh);
        let stream = |b: Big| pack.get(b.off as usize..(b.off + b.len) as usize);
        for size in Size::ALL {
            let pic = m.make(1, &index, size, 4, pics, stream).expect("made");
            let want = if size == Size::Hero {
                (w, h)
            } else {
                rp_draw::shrink::size(w, h, size.side())
            };
            assert_eq!((pic.w, pic.h), want, "{size:?}");
            if size == Size::Hero {
                assert_eq!(pics.read(pic, <[u32]>::to_vec), Some(a.clone()));
            } else {
                // Opaque in, opaque out; the gradient kept in order.
                let px = pics.read(pic, <[u32]>::to_vec).unwrap();
                assert!(px.iter().all(|p| p >> 24 == 0xFF));
                assert!(px[0] >> 16 & 0xFF < px[usize::from(pic.w) - 1] >> 16 & 0xFF);
            }
        }
        assert_eq!(m.inflates, 1, "one inflate for four sizes");
        // Another pack's tag inflates again; a condition it lacks is none.
        assert!(m.make(2, &index, Size::Row, 4, pics, stream).is_some());
        assert_eq!(m.inflates, 2);
        assert!(m.make(2, &index, Size::Row, 6, pics, stream).is_none());
        // A damaged stream: none, never a fault, and nothing held.
        let mut bad = pack.clone();
        let b4 = index.big(4).unwrap();
        bad[(b4.off + b4.len / 2) as usize] ^= 0x5A;
        m.forget();
        let s2 = |b: Big| bad.get(b.off as usize..(b.off + b.len) as usize);
        assert!(m.make(3, &index, Size::Card, 4, pics, s2).is_none());
        // Tables that lie: a picture over the hero's box, an id past 22.
        let mut over = pack.clone();
        over[PACK_HEAD + 2..PACK_HEAD + 4].copy_from_slice(&251u16.to_le_bytes());
        assert!(PackIndex::read(&over).is_none());
        let mut tall = pack.clone();
        tall[PACK_HEAD + 4..PACK_HEAD + 6].copy_from_slice(&177u16.to_le_bytes());
        assert!(
            PackIndex::read(&tall).is_none(),
            "taller than the hero's box"
        );
        let mut id = pack.clone();
        id[PACK_HEAD + 1] = 23;
        assert!(PackIndex::read(&id).is_none());
        assert!(PackIndex::read(&pack[..PACK_HEAD + 5]).is_none());
    }

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

    /// A store with a slot for every condition at every size (leaked: it
    /// lives for the test, as the image's lives for the run).
    fn host_store() -> &'static Pics<'static> {
        let classes: Vec<(u16, u16, u8)> =
            Size::ALL.iter().map(|z| (z.side(), z.side(), 22)).collect();
        Box::leak(Box::new(Pics::new(
            Box::leak(vec![0; Pics::words(&classes)].into_boxed_slice()),
            &classes,
        )))
    }

    /// A pack of three conditions' big pictures (gradients of their own),
    /// every size made by the maker into a store of its own (handed to the
    /// renderers that draw them), as the image makes them.
    fn host_icons_in() -> (rp_shell::Icons, &'static Pics<'static>) {
        let big = |id: u32, w: u16, h: u16| -> Vec<u32> {
            (0..u32::from(w) * u32::from(h))
                .map(|i| {
                    0xFF00_0000 | (i % u32::from(w)) << 16 | (i / u32::from(w)) << 8 | (id * 40)
                })
                .collect()
        };
        let pack = v2_pack(&[
            (0, 3, 250, 174, big(3, 250, 174)),
            (0, 4, 200, 176, big(4, 200, 176)),
            (0, 17, 176, 176, big(17, 176, 176)),
        ]);
        let pics = host_store();
        let mut b = vec![0; BIG_WORDS];
        let mut sh = vec![0; Maker::shrink_words()];
        let icons = make_all(&pack, pics, &mut Maker::new(&mut b, &mut sh));
        (icons, pics)
    }

    fn host_icons() -> rp_shell::Icons {
        host_icons_in().0
    }

    /// The Keeper makes what the screens show and nothing else, a picture
    /// a step; a new pack replaces each in turn, the old one shown until
    /// then; what no screen shows any more is given back; a picture the
    /// pack lacks is tried once.
    #[test]
    fn the_keeper_keeps_exactly_what_the_screens_show() {
        let pics = host_store();
        let mut shell = rp_shell::Shell::new(shell_fonts(), rp_shell::Icons::NONE, true);
        let want = shell.wanted();
        let img: &'static Image = Box::leak(Box::new(Image {
            w: 2,
            h: 2,
            px: Box::leak(vec![0xFF00_0000; 4].into_boxed_slice()),
        }));
        let mut k = Keeper::new();
        let mut made = 0;
        let mut order = Vec::new();
        let pack_of = |pack: u32| {
            move |_: Size, _: usize, p: &Pics| {
                let pic = store_copy(img, p)?;
                p.write(pic, |px| px[0] = pack);
                Some(pic)
            }
        };
        loop {
            match k.step(&mut shell, pics, 0, pack_of(0)) {
                Kept::Made(z, _) => {
                    made += 1;
                    order.push(z);
                }
                Kept::Idle => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(made, want.count());
        assert_eq!(pics.used() as u32, want.count());
        // The Today screen's pictures (big, cards) before the app's.
        let today = order
            .iter()
            .take_while(|z| matches!(z, Size::Hero | Size::Card))
            .count();
        assert_eq!(
            today as u32,
            want.0[0].count_ones() + want.0[2].count_ones()
        );
        for size in Size::ALL {
            for i in 0..22 {
                assert_eq!(shell.pic(size, i).is_some(), want.has(size, i));
            }
        }
        // Pack 1: each replaced in turn, the rest still pack 0's.
        let hero = (0..22).find(|&i| want.has(Size::Hero, i)).unwrap();
        let old = shell.pic(Size::Hero, hero).unwrap();
        assert_eq!(
            k.step(&mut shell, pics, 1, pack_of(1)),
            Kept::Made(Size::Hero, hero)
        );
        let new = shell.pic(Size::Hero, hero).unwrap();
        assert!(!pics.is_live(old) && pics.is_live(new));
        assert_eq!(pics.read(new, |px| px[0]), Some(1));
        let mut n = 1;
        while k.step(&mut shell, pics, 1, pack_of(1)) != Kept::Idle {
            n += 1;
        }
        assert_eq!(n, want.count());
        assert_eq!(pics.used() as u32, want.count(), "every old one given back");
        // A forecast with other skies: what no screen shows goes.
        let mut wx = rp_weather::sample();
        wx.now.sky = wx.days[3].sky;
        for h in &mut wx.hours {
            h.sky = wx.now.sky;
        }
        shell.set_forecast(wx);
        let mut freed = 0;
        loop {
            match k.step(&mut shell, pics, 1, pack_of(1)) {
                Kept::Freed(..) => freed += 1,
                Kept::Idle => break,
                _ => {}
            }
        }
        assert!(freed > 0);
        assert_eq!(pics.used() as u32, shell.wanted().count());
        // A pack without pictures: tried once each, none, never a loop.
        let mut misses = 0;
        loop {
            match k.step(&mut shell, pics, 2, |_, _, _| None) {
                Kept::Missing(..) => misses += 1,
                Kept::Idle => break,
                _ => {}
            }
        }
        assert_eq!(misses, shell.wanted().count());
        assert_eq!(pics.used(), 0);
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
    fn the_pictures_filter_undoes_exactly() {
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
        let (icons, pics) = host_icons_in();
        gpu.set_pics(pics);
        let mut shell = Shell::new(shell_fonts(), icons, true);
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
            let mut cpu = CpuRenderer::new(layer, &mut plane, STRIDE, PlaneFormat::Argb8888);
            cpu.set_pics(pics);
            cpu.draw(&f);
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
        let (icons, pics) = host_icons_in();
        gpu.set_pics(pics);
        let mut shell = Shell::new(shell_fonts(), icons, true);
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

    /// Hourly's card with the real fonts: every line ends inside the card
    /// for every sky and the widest readings (three-digit temperatures,
    /// gusts, km/h), the text moved right for the 160 px picture (the
    /// owner's pick, 2026-10-31).
    #[test]
    fn every_hour_card_line_fits_with_the_wide_picture() {
        use rp_draw::{Cmd, Frame};
        use rp_shell::{Input, Key, Shell};
        for metric in [false, true] {
            for wmo in [0u8, 2, 3, 45, 51, 61, 65, 66, 71, 75, 77, 80, 85, 95, 99] {
                let mut wx = rp_weather::sample();
                let h = &mut wx.hours[0];
                h.sky.wmo = wmo;
                h.sky.wind = 1_999;
                (h.temp, h.feels, h.dew) = (-499, -499, -499);
                (h.humidity, h.precip, h.uv) = (100, 100, 11);
                (h.gust, h.rain, h.vis) = (1_999, 9_999, 65_000);
                h.wind_from = 337;
                let mut s = Shell::new(shell_fonts(), rp_shell::Icons::NONE, true);
                s.set_forecast(wx);
                if metric {
                    s.set_region(rp_weather::Region::METRIC);
                }
                for e in [
                    Input::PenDown { x: 240, y: 150 },
                    Input::PenUp { x: 240, y: 150 },
                    Input::Key(Key::Right),
                ] {
                    s.input(e);
                    s.settle();
                }
                let mut f = Frame::new();
                s.frame(&mut f);
                let mut lines = 0;
                for c in f.cmds() {
                    // The card's lines (326 to 522 on the page, which
                    // starts at the screen's row 0 at rest), not the chrome.
                    if let Cmd::Text {
                        x,
                        baseline,
                        font,
                        text,
                        ..
                    } = *c
                        && (326..522).contains(&baseline)
                    {
                        lines += 1;
                        let end = x + font.width(f.text_of(text));
                        assert!(
                            end <= 464 - 8,
                            "wmo {wmo}, metric {metric}: {:?} ends at {end}",
                            core::str::from_utf8(f.text_of(text))
                        );
                    }
                }
                assert!(
                    lines >= 6,
                    "wmo {wmo}: the card's time and five lines seen ({lines})"
                );
            }
        }
    }
}
