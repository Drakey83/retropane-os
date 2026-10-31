//! The page layer (ADR-0034 C4 as changed, L2; sheet
//! `docs/research/sheets/weather-gpu-movement.md`, "The plane path build, as
//! built"): a page's glass and text drawn once into an ARGB4444 picture (V3)
//! with per-pixel alpha, shown on the video plane over a still screen, and
//! moved by repointing the plane in the blanking: no engine runs a frame.
//!
//! - **One atomic commit a refresh** (the modern way: Linux's atomic mode
//!   setting). The layer's picture, window and on/off, and the 2D
//!   renderer's held flip, are staged and written together in scan lines
//!   643-2 (run WG on the X51v: window repoints there whole every refresh,
//!   gpubench-0248/-0249/-0251), the changed registers only. A newer window
//!   replaces a staged one: the latest wins, never a queue of stale frames.
//!   A hand-off between the composed screen and the screen with its layer
//!   is one commit, so it lands in one refresh (M4).
//! - **Pictures by handle** (ADR-0030 G3, ADR-0027): pool blocks under the
//!   client's quota; a window reading outside its picture or starting off
//!   the plane's 16-byte grid is refused, never shown.
//! - **Amendment P by construction** (ADR-0034 rules 2 and 4): the layer's
//!   pictures lie outside the safe region, so the layer can't show while a
//!   3D scene is open, nor a scene open while it shows.
//! - **An app composed aside** ([`Source::Aside`]): a 2D frame drawn into
//!   the picture not on screen and shown only through the layer's window
//!   (RGB565, opaque): Reveal's app growing out of the Today card, handed
//!   over by one commit that shows the same pixels on the graphics plane.

use rp_draw::{Frame, Renderer as _};
use rp_drv_mbx_display::layers::{self, Blend, Layer, Scale, Slot, VideoFormat, VideoPlane};
use rp_hal_api::{Field, RegisterBus};
use rp_mbx_render::LocalMem;
use rp_mbx3d_render::Clock;
use rp_mbx3d_render::scan;

use super::{ClientId, Error, FLIP_LEAD_US, Graphics, Handle, Job, Step};

/// Page pictures at once, all clients together (a page, the next page, a
/// turn's strip, the radar's two).
pub const PICTURES: usize = 6;

/// The first scan line of the plane window: lines 643-644 and 0-2 of the
/// 645-line refresh (GC2DC run 2: plane writes clean there; run WG wrote
/// every window there, whole).
const PLANE_FROM: u32 = 643;
/// Its last line, after the wrap.
const PLANE_TO: u32 = 2;
/// The plane window's lines.
const PLANE_LINES: u32 = 5;
/// The screens' stride (RGB565, 480 wide): an app composed aside.
const SCREEN_STRIDE: u32 = 960;

/// Whether `line` is in the plane window.
const fn in_window(line: u32) -> bool {
    line >= PLANE_FROM || line <= PLANE_TO
}

/// Whether a commit may start on `line`: in the window but not its last
/// line, so its few writes end inside it (the X51v's first run of the app:
/// 17 of 1,309 commits ended outside).
const fn may_start(line: u32) -> bool {
    line >= PLANE_FROM || line < PLANE_TO
}

/// Where the layer shows on screen and what of its picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// The window's left edge on screen.
    pub x: u16,
    /// Its top edge.
    pub y: u16,
    /// Its width (even).
    pub w: u16,
    /// Its height (even).
    pub h: u16,
    /// The picture's row shown at the window's top.
    pub row: u16,
    /// The picture's column shown at its left (on the plane's 16-byte grid:
    /// a multiple of 8 pixels at 2 bytes a pixel).
    pub col: u16,
}

/// What the layer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// One of the client's page pictures (ARGB4444, per-pixel alpha).
    Picture(Handle),
    /// The 2D frame composed aside (RGB565, opaque), while one is held
    /// ([`Graphics::frame_commit`] with `aside`).
    Aside,
}

/// The layer's state: its source and window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerState {
    /// What it shows.
    pub src: Source,
    /// Where.
    pub win: Window,
}

/// A page picture's record.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pic {
    live: bool,
    owner: u8,
    generation: u16,
    at: u32,
    w: u16,
    h: u16,
}

/// The next commit: the layer as it should be from the next blanking (its
/// owner), and whether the held 2D frame is flipped with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Staged {
    layer: Option<(LayerState, u8)>,
    flip: bool,
    /// The layer's registers, worked out when staged (the blanking then
    /// only writes them); `None` until an aside source has landed.
    regs: Option<[(usize, u32); 9]>,
}

/// The layer's own state in the service.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LayerRec {
    pics: [Pic; PICTURES],
    /// The layer on screen and its owner.
    shown: Option<(LayerState, u8)>,
    /// Its nine registers' values as last written (in the driver's order).
    regs: [u32; 9],
    /// The commit waiting for the blanking.
    staged: Option<Staged>,
    /// A 2D frame held aside (shown only through the layer).
    aside: bool,
}

/// The layer's counts, for the logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Commits {
    /// Commits written.
    pub done: u32,
    /// ... of them, ending outside the plane window (may have torn).
    pub late: u32,
    /// The scan line the last commit ended on.
    pub last_line: Option<u32>,
    /// Registers written by the last commit.
    pub last_regs: u32,
}

/// Bytes a line of a `w`-pixel ARGB4444 picture, on the plane's 16-byte
/// grid.
const fn stride(w: u16) -> u32 {
    (w as u32 * 2).div_ceil(16) * 16
}

impl<R: RegisterBus, M: LocalMem> Graphics<'_, R, M> {
    /// A `w` x `h` ARGB4444 page picture for client `c`, within its quota
    /// (cleared to transparent by the caller's first write).
    pub fn picture(&mut self, c: ClientId, w: u16, h: u16) -> Result<Handle, Error> {
        let rec = self.rec(c)?;
        if !(2..=2048).contains(&w) || !(2..=2048).contains(&h) {
            return Err(Error::BadWindow);
        }
        let bytes = stride(w) * u32::from(h);
        if rec.used.saturating_add(bytes) > rec.quota {
            return Err(Error::Quota);
        }
        let k = self
            .layer
            .pics
            .iter()
            .position(|p| !p.live)
            .ok_or(Error::NoMemory)?;
        let at = self.pool.alloc(bytes, 16).ok_or(Error::NoMemory)?;
        let p = &mut self.layer.pics[k];
        *p = Pic {
            live: true,
            owner: c.0,
            generation: p.generation,
            at,
            w,
            h,
        };
        self.clients[usize::from(c.0)].used += bytes;
        Ok(Handle {
            slot: k as u16,
            generation: p.generation,
        })
    }

    /// Client `c`'s picture behind `h`.
    fn pic(&self, c: u8, h: Handle) -> Result<Pic, Error> {
        match self.layer.pics.get(usize::from(h.slot)) {
            Some(p) if p.live && p.owner == c && p.generation == h.generation => Ok(*p),
            _ => Err(Error::BadHandle),
        }
    }

    /// A picture's size and stride (bytes a line).
    pub fn picture_size(&self, c: ClientId, h: Handle) -> Result<(u16, u16, u32), Error> {
        let p = self.pic(c.0, h)?;
        Ok((p.w, p.h, stride(p.w)))
    }

    /// Writes `words` (two ARGB4444 pixels each, the left one low) into
    /// client `c`'s picture `h` from byte `offset` on, rows in order at its
    /// stride. Refused if they'd run past its end.
    pub fn write_picture(
        &mut self,
        c: ClientId,
        h: Handle,
        offset: u32,
        words: &[u32],
    ) -> Result<(), Error> {
        let p = self.pic(c.0, h)?;
        let end = u64::from(offset) + 4 * words.len() as u64;
        if !offset.is_multiple_of(4) || end > u64::from(stride(p.w)) * u64::from(p.h) {
            return Err(Error::BadWindow);
        }
        let (_, mem) = self.two_d.parts();
        mem.store((p.at + offset) as usize, words);
        mem.sync();
        Ok(())
    }

    /// Gives a page picture back; refused while the layer shows it or a
    /// commit is about to.
    pub fn free_picture(&mut self, c: ClientId, h: Handle) -> Result<(), Error> {
        self.rec(c)?;
        let p = self.pic(c.0, h)?;
        let uses = |s: Option<(LayerState, u8)>| {
            s.is_some_and(|(st, o)| o == c.0 && st.src == Source::Picture(h))
        };
        if uses(self.layer.shown) || self.layer.staged.is_some_and(|s| uses(s.layer)) {
            return Err(Error::Busy);
        }
        let bytes = stride(p.w) * u32::from(p.h);
        let freed = self.pool.free(p.at, bytes);
        debug_assert!(freed, "page picture not in the pool");
        let owner = usize::from(c.0);
        self.clients[owner].used = self.clients[owner].used.saturating_sub(bytes);
        let r = &mut self.layer.pics[usize::from(h.slot)];
        r.live = false;
        r.generation = r.generation.wrapping_add(1);
        Ok(())
    }

    /// Client `c`'s pictures given back (closing it); `Busy` if the layer
    /// shows one of them.
    pub(crate) fn free_pictures(&mut self, c: ClientId) -> Result<(), Error> {
        for k in 0..PICTURES {
            let p = self.layer.pics[k];
            if p.live && p.owner == c.0 {
                self.free_picture(
                    c,
                    Handle {
                        slot: k as u16,
                        generation: p.generation,
                    },
                )?;
            }
        }
        Ok(())
    }

    /// The source's place, stride, format and blend, or why not.
    fn source(
        &mut self,
        owner: u8,
        src: Source,
    ) -> Result<(u32, u32, u16, u16, VideoFormat, Blend), Error> {
        match src {
            Source::Picture(h) => {
                let p = self.pic(owner, h)?;
                Ok((
                    p.at,
                    stride(p.w),
                    p.w,
                    p.h,
                    VideoFormat::Argb4444,
                    Blend::PixelAlpha,
                ))
            }
            Source::Aside => {
                let at = match self.two_d.landed() {
                    Some(at) if self.layer.aside => at as u32,
                    _ => return Err(Error::BadWindow),
                };
                Ok((
                    at,
                    SCREEN_STRIDE,
                    480,
                    640,
                    VideoFormat::Rgb565,
                    Blend::Opaque,
                ))
            }
        }
    }

    /// The layer's nine registers for `st` (client `owner`'s), or why the
    /// window can't be shown: odd or empty sizes, off the screen, outside
    /// its picture, or a start off the 16-byte grid.
    fn registers(&mut self, owner: u8, st: LayerState) -> Result<[(usize, u32); 9], Error> {
        let (base, stride, pw, ph, fmt, blend) = self.source(owner, st.src)?;
        let w = st.win;
        let fits = (2..=480).contains(&w.w)
            && (2..=640).contains(&w.h)
            && w.w.is_multiple_of(2)
            && w.h.is_multiple_of(2)
            && u32::from(w.x) + u32::from(w.w) <= 480
            && u32::from(w.y) + u32::from(w.h) <= 640
            && u32::from(w.col) + u32::from(w.w) <= u32::from(pw)
            && u32::from(w.row) + u32::from(w.h) <= u32::from(ph)
            && (u32::from(w.col) * 2).is_multiple_of(16);
        if !fits {
            return Err(Error::BadWindow);
        }
        // The look's gamma as the screen has it (the owner's picture,
        // display-calibration.md): the video plane's own bit follows the
        // graphics plane's, so a page on the layer and the screen under it
        // go through the same curve.
        let gamma = self
            .two_d
            .parts()
            .0
            .regs
            .read32(rp_drv_mbx_display::reg::GSCTRL)
            >> 26
            & 1
            != 0;
        let mut v = VideoPlane::new(fmt, w.w, w.h, 0);
        v.offset = (base + u32::from(w.row) * stride + 2 * u32::from(w.col)) as usize;
        v.stride = stride as usize;
        let scale = Scale {
            x: w.x,
            y: w.y,
            ..Scale::NONE
        };
        Ok(v.registers_scaled(
            Layer {
                slot: Slot::Middle,
                blend,
            },
            gamma,
            scale,
        ))
    }

    /// Whether a scene is open or about to be: the layer can't show then.
    fn scene_open(&self) -> bool {
        self.open().is_some()
    }

    /// Starts a commit: the 2D `frame` drawn (into the picture not on
    /// screen), then, in the next plane window once it's done, shown (or
    /// kept aside, shown only through the layer, if `aside`) together with
    /// the layer set to `layer` (`None`: off). A hand-off between the
    /// composed screen and screen + layer in one refresh. [`Self::poll`]
    /// drives it.
    pub fn frame_commit<C: Clock>(
        &mut self,
        c: ClientId,
        frame: &Frame,
        layer: Option<LayerState>,
        aside: bool,
        clock: &mut C,
    ) -> Result<Step, Error> {
        self.rec(c)?;
        if self.job != Job::None || self.layer.aside || self.layer.staged.is_some_and(|s| s.flip) {
            return Err(Error::Busy);
        }
        if layer.is_some() && self.scene_open() {
            return Err(Error::LayerAnd3d);
        }
        // An aside source shows the frame this commit draws: checked once
        // it has landed. Any other is checked now.
        let mut regs = None;
        if let Some(st) = layer
            && !(aside && st.src == Source::Aside)
        {
            regs = Some(self.registers(c.0, st)?);
        }
        if aside && self.copies.band().is_none() {
            // As `frame`: a copy's time known first (it copies between the
            // screens, so before anything is held aside).
            self.time_copy(clock);
        }
        self.layer.staged = Some(Staged {
            layer: layer.map(|st| (st, c.0)),
            flip: false,
            regs,
        });
        self.two_d.draw(frame);
        if self.two_d.landed().is_some() || !self.two_d.poll() {
            self.job = Job::Commit { aside };
        }
        Ok(self.poll(clock))
    }

    /// The 2D part of a commit landed: kept aside, or flipped with the
    /// layer.
    pub(crate) fn commit_landed(&mut self, aside: bool) -> bool {
        if self.two_d.landed().is_none() {
            let _ = self.two_d.poll();
            if self.two_d.landed().is_none() {
                return false;
            }
        }
        self.job = Job::None;
        if aside {
            self.layer.aside = true;
            // The aside source now exists: its registers worked out.
            if let Some(mut s) = self.layer.staged
                && let Some((st, owner)) = s.layer
            {
                s.regs = self.registers(owner, st).ok();
                self.layer.staged = Some(s);
            }
        } else if let Some(s) = self.layer.staged.as_mut() {
            s.flip = true;
        }
        true
    }

    /// The frame held aside shown on the graphics plane, and the layer set
    /// to `layer` (`None`: off), in one commit: the end of Reveal (the same
    /// pixels both sides).
    pub fn show_aside(&mut self, c: ClientId, layer: Option<LayerState>) -> Result<(), Error> {
        self.rec(c)?;
        if !self.layer.aside || self.job != Job::None {
            return Err(Error::Busy);
        }
        let mut regs = None;
        if let Some(st) = layer {
            if st.src == Source::Aside {
                return Err(Error::BadWindow);
            }
            regs = Some(self.registers(c.0, st)?);
        }
        self.layer.staged = Some(Staged {
            layer: layer.map(|st| (st, c.0)),
            flip: true,
            regs,
        });
        Ok(())
    }

    /// Moves the layer's window (its source kept) at the next blanking: a
    /// drag, a fling, an entrance. A window already staged is replaced.
    pub fn move_layer(&mut self, c: ClientId, win: Window) -> Result<(), Error> {
        self.rec(c)?;
        let staged = self.layer.staged;
        let (cur, owner) = match staged.map(|s| s.layer) {
            Some(l) => l,
            None => self.layer.shown,
        }
        .ok_or(Error::BadWindow)?;
        if owner != c.0 {
            return Err(Error::BadHandle);
        }
        let st = LayerState { src: cur.src, win };
        // An aside source waiting for its frame to land is checked then.
        let mut regs = None;
        if !(st.src == Source::Aside && !self.layer.aside) {
            regs = Some(self.registers(owner, st)?);
        }
        self.layer.staged = Some(Staged {
            layer: Some((st, owner)),
            flip: staged.is_some_and(|s| s.flip),
            regs,
        });
        Ok(())
    }

    /// The layer on screen (`None`: off).
    #[must_use]
    pub fn layer(&self) -> Option<LayerState> {
        self.layer.shown.map(|(st, _)| st)
    }

    /// Whether a commit waits for the blanking (the caller wakes for it:
    /// [`Self::poll`] says when).
    #[must_use]
    pub fn commit_pending(&self) -> bool {
        self.layer.staged.is_some() || matches!(self.job, Job::Commit { .. })
    }

    /// Whether a 2D frame is held aside.
    #[must_use]
    pub const fn aside_held(&self) -> bool {
        self.layer.aside
    }

    /// The commits so far.
    #[must_use]
    pub const fn commits(&self) -> Commits {
        self.commits
    }

    /// Drives the frame in flight and the staged commit.
    pub fn poll<C: Clock>(&mut self, clock: &mut C) -> Step {
        let job = self.poll_job(clock);
        if self.layer.staged.is_none() || matches!(self.job, Job::Commit { .. }) {
            return job;
        }
        let commit = self.poll_commit(clock);
        match (job, commit) {
            (Step::Wait(a), Step::Wait(b)) => {
                // The sooner, on the clock's wrapping count.
                let now = clock.now_us();
                if a.wrapping_sub(now) <= b.wrapping_sub(now) {
                    Step::Wait(a)
                } else {
                    Step::Wait(b)
                }
            }
            (Step::Wait(t), Step::Shown) | (Step::Shown, Step::Wait(t)) => Step::Wait(t),
            (Step::Idle, c) => c,
            (j, _) => j,
        }
    }

    /// The staged commit written once the scan is in the plane window (the
    /// CPU waiting at most [`FLIP_LEAD_US`] for it), else a wait until it
    /// nears. A clock with no measured scan commits at once.
    fn poll_commit<C: Clock>(&mut self, clock: &mut C) -> Step {
        if let Some(sc) = self.scan() {
            let lead = FLIP_LEAD_US.saturating_mul(1000).div_ceil(sc.line_ns);
            let t0 = clock.now_us();
            loop {
                let line = self.scan_line();
                if may_start(line) {
                    break;
                }
                let until = (PLANE_FROM + sc.total - line % sc.total) % sc.total;
                if until > lead || clock.now_us().wrapping_sub(t0) > 2 * FLIP_LEAD_US {
                    let early = (PLANE_FROM + sc.total - lead.min(until)) % sc.total;
                    return Step::Wait(self.when_line(early, clock));
                }
                clock.delay_us(5);
            }
        }
        self.apply_commit();
        Step::Shown
    }

    /// Writes the staged commit: the held frame flipped, then the layer's
    /// changed registers in the driver's order (the stream bit last).
    fn apply_commit(&mut self) {
        let Some(s) = self.layer.staged.take() else {
            return;
        };
        // Worked out when staged; only an aside source not yet landed then
        // is worked out here (never shown half-checked: off if it can't be).
        let want = match s.layer {
            Some((st, owner)) => match s.regs {
                Some(r) => Some((r, st, owner)),
                None => self.registers(owner, st).ok().map(|r| (r, st, owner)),
            },
            None => None,
        };
        if s.flip {
            self.two_d.flip();
            self.layer.aside = false;
        }
        let mut n = u32::from(s.flip);
        let was_on = self.layer.shown.is_some();
        let regs = &mut self.two_d.parts().0.regs;
        match want {
            Some((r, st, owner)) => {
                if !was_on {
                    regs.modify(layers::reg::VOVRCLK, Field::bit(0), 1);
                }
                for (k, &(off, v)) in r.iter().enumerate() {
                    if !was_on || self.layer.regs[k] != v {
                        regs.write32(off, v);
                        self.layer.regs[k] = v;
                        n += 1;
                    }
                }
                self.layer.shown = Some((st, owner));
            }
            None => {
                if was_on {
                    rp_drv_mbx_display::video_hide(regs);
                    n += 1;
                }
                self.layer.shown = None;
                self.layer.regs = [0; 9];
            }
        }
        let line = self.scan_line();
        self.commits.done += 1;
        self.commits.late += u32::from(self.scan().is_some() && !in_window(line));
        self.commits.last_line = Some(line);
        self.commits.last_regs = n;
    }
}

/// The plane window is five lines long (the scan model's check).
const _: () = assert!((scan::X51V_60HZ.total - PLANE_FROM) + PLANE_TO + 1 == PLANE_LINES);

#[cfg(test)]
pub(crate) const fn window_line(line: u32) -> bool {
    in_window(line)
}

#[cfg(test)]
impl<R: RegisterBus, M: LocalMem> Graphics<'_, R, M> {
    /// A picture's place in 2700G memory (tests).
    pub(crate) fn picture_at(&self, c: ClientId, h: Handle) -> Result<u32, Error> {
        self.pic(c.0, h).map(|p| p.at)
    }
}
