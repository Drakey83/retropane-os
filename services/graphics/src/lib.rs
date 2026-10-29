//! The graphics service (ADR-0030; ADR-0026 D6): the one owner of the
//! 2700G's 2D engine and 3D core.
//!
//! - **One queue** (G2): one job at a time, a 2D frame or a 3D render; a 3D
//!   render waits for the 2D engine to be idle, a 2D frame for the 3D
//!   render in flight.
//! - **Resets coordinated:** a 3D recovery resets the whole MBX, so the 2D
//!   engine's stream restarts after it; the 3D clock goes off when idle.
//! - **Clients get handles, not addresses** (G3): a client's textures are
//!   blocks from the 2700G memory pool (ADR-0027) under its quota; its
//!   quads name them by handle, so a client can only ever point the GPU at
//!   its own memory. Refusals are reported, nothing is evicted.
//! - **One 3D scene** (a renderer drives the core at a time), owned by one
//!   client, with **its own area of the screen** (G1, option A, the
//!   owner's vote), the scene redrawing the tiles 2D drew over.
//! - **The 3D core draws only into screen 0, with screen 0 on screen**
//!   (C71, the owner's ruling R-B, 2026-10-29; ADR-0034 amendment P, P1 =
//!   C): screen 0 lies inside the safe region, screen 1 outside it as 2D's
//!   back picture. 2D frames flip between the two as ever (tear-free). In a
//!   frame with 3D work, 2D's frame reaches screen 0 first (a flip if 2D
//!   drew there, else its changed rectangles copied in by the 2D engine),
//!   the scene renders into screen 0 on screen, and its area is copied back
//!   into screen 1.
//! - **Tear-free 3D frames** (R20, ADR-0034 P1 = C with the owner's
//!   fallback, "Option 1", 2026-10-29): each scene frame is planned from
//!   its recent render times (`rp_mbx3d_render::scan::Schedule`) on the
//!   display's measured scan (R111). One the scan can lead (a top-down
//!   order, times that fit) starts on screen 0 at a line inside its window,
//!   behind the scan. One it can't (no history, a spike, a checkerboard, too
//!   long) renders into screen 1 while screen 0 is shown, and the 2D engine
//!   copies its area into screen 0 behind the scan, planned like a render
//!   from the copies' own measured times (the owner's ruling 2026-10-29,
//!   "Yes, copy variant": no flip, so the 3D core is free once the copy
//!   ends; [`Graphics::allow_screen1_3d`], once the soak of 3D targets at
//!   screen 1's place has passed on the X51v). Without that, or before any
//!   copy has been timed, it starts on screen 0 once the scan has passed
//!   the scene's bottom row, by the refresh's last line.
//!
//! A library now (G4), used in one process; the display driver host's once
//! IPC can move pages (ADR-0016).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_draw::{Frame, MAX_DIRTY, Rect, Renderer as _};
use rp_drv_mbx_display::{pll, reg as dreg};
use rp_hal_api::RegisterBus;
use rp_mbx_render::{LocalMem, Mbx2dRenderer};
use rp_mbx3d_render::scan::{self, Scan, Schedule};
use rp_mbx3d_render::{
    self as r3d, AlphaTest, Clock, Depth, Draw, Paint, Picture, Point, Poll, Target, Texture,
};
use rp_vram_map::Pool;

/// Clients at once.
pub const CLIENTS: usize = 8;
/// Blocks (textures) at once, all clients together.
pub const BLOCKS: usize = 64;

/// A client of the service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientId(u8);

/// A client's block of 2700G memory: meaningful only to that client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    slot: u16,
    generation: u16,
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The other engine (or this one) has work in flight: poll first.
    Busy,
    /// No such client, or no room for another.
    NoClient,
    /// Over the client's quota.
    Quota,
    /// The pool (or the block table) is full.
    NoMemory,
    /// Not one of this client's blocks.
    BadHandle,
    /// The 3D scene is another client's, or none is open.
    NotYourScene,
    /// The program gave the service no 3D state ([`ThreeD`]): it draws 2D
    /// only.
    No3d,
    /// Refused by the 3D renderer.
    Scene(r3d::Error),
}

/// How a quad is filled, its texture by handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// One colour.
    Flat(u32),
    /// A colour per corner.
    Gouraud([u32; 4]),
    /// One of the client's textures.
    Texture {
        /// The client's handle for it.
        tex: Handle,
        /// Coordinates per corner (16.16).
        uv: [(i32, i32); 4],
        /// Colour the texels are multiplied by.
        tint: u32,
        /// Across: `rp_mbx3d_render::address`.
        u_mode: u32,
        /// Down.
        v_mode: u32,
        /// Bilinear filtering (smooth when scaled), or point sampling.
        linear: bool,
    },
}

impl Fill {
    /// The whole of `tex`, once, untinted, clamped.
    #[must_use]
    pub const fn texture(tex: Handle) -> Self {
        Self::Texture {
            tex,
            uv: [
                (0, 0),
                (r3d::UV_ONE, 0),
                (r3d::UV_ONE, r3d::UV_ONE),
                (0, r3d::UV_ONE),
            ],
            tint: 0xFFFF_FFFF,
            u_mode: r3d::address::CLAMP,
            v_mode: r3d::address::CLAMP,
            linear: false,
        }
    }
}

/// A client's quad: [`Draw`] with its texture by handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quad {
    /// Top left, top right, bottom right, bottom left (1/16 pixel).
    pub corners: [Point; 4],
    /// Its fill.
    pub fill: Fill,
    /// Blend factors, or opaque.
    pub blend: Option<(u32, u32)>,
    /// A depth test, or none.
    pub depth: Option<Depth>,
    /// An alpha test (cut-out shapes), or none.
    pub alpha: Option<AlphaTest>,
}

impl Quad {
    /// A `w` x `h` pixel rectangle at `(x, y)`.
    #[must_use]
    pub const fn rect(x: i32, y: i32, w: i32, h: i32, fill: Fill) -> Self {
        let d = Draw::rect(x, y, w, h, Paint::Flat(0));
        Self {
            corners: d.corners,
            fill,
            blend: None,
            depth: None,
            alpha: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ClientRec {
    live: bool,
    quota: u32,
    used: u32,
}

#[derive(Clone, Copy, Debug)]
struct Block {
    owner: u8,
    generation: u16,
    tex: Option<Texture>,
}

const NO_BLOCK: Block = Block {
    owner: 0,
    generation: 0,
    tex: None,
};

/// The 3D scene: its owner, its renderer, its area of the screen.
struct Scene {
    owner: u8,
    r: r3d::Renderer,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    /// Its renderer's core resets already passed on to the 2D engine.
    resets: u32,
}

/// Tests: run on the 2D renderer just before a copy between the screens.
#[cfg(test)]
type CopyHook<R, M> = fn(&mut Mbx2dRenderer<R, M>);

/// The job in flight (one at a time, G2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    None,
    /// A 2D frame, its flip held for the 3D area.
    TwoD,
    /// The 3D area waiting to start on screen 0, on screen, at a scan line
    /// in `from..=to` (`to` past the refresh's last line runs into the next
    /// refresh's first): behind the scan (`behind`), or once the scan has
    /// left the area. `prepared`: its lists written already (else written
    /// at the start: the core needed setting up).
    Armed {
        from: u32,
        to: u32,
        behind: bool,
        prepared: bool,
    },
    /// The 3D area rendering into screen 0, on screen (C71 ruling R-B);
    /// then the area carried into screen 1.
    ThreeD,
    /// R20's fallback: the 3D area rendering into screen 1 while screen 0
    /// is shown; `held`: 2D's frame is held in screen 1 (its changes go
    /// into screen 0 with the area). The copy starts at a line in
    /// `from..=to`.
    Aside {
        held: bool,
        from: u32,
        to: u32,
    },
    /// The fallback's copy into screen 0 (on screen) behind the scan,
    /// waiting for a line in `from..=to`.
    Copy {
        held: bool,
        from: u32,
        to: u32,
    },
}

/// How the 3D frames started (R20), for the logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Starts {
    /// On screen 0 behind the scan (C).
    pub behind: u32,
    /// On screen 0 once the scan had passed the scene's bottom row (the
    /// fallback until screen 1's soak passes and a copy has been timed).
    pub after_area: u32,
    /// Into screen 1, then copied into screen 0 behind the scan.
    pub screen1: u32,
    /// The scan line the last frame on screen 0 started on.
    pub last_line: Option<u32>,
    /// The scan line the last fallback copy started on.
    pub last_copy_line: Option<u32>,
}

/// How a scene frame starts (R20).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    /// On screen 0 at a line in `from..=to` (`behind` the scan, or after the
    /// area).
    Screen0 { from: u32, to: u32, behind: bool },
    /// Into screen 1, copied into screen 0 from a line in `from..=to`.
    Screen1 { from: u32, to: u32 },
}

/// Lines after the scene's area in which a fallback frame may still start
/// on screen 0 (two tile rows: the caller's poll may come late), never
/// past the refresh's last line (the owner's direction, 2026-10-29: a
/// full-screen scene starts in lines 640-644).
const AFTER_AREA_LINES: u32 = 2 * scan::MARGIN;

/// What [`Graphics::poll`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing in flight.
    Idle,
    /// Not done: poll again at this time (microseconds, the clock's).
    Wait(u32),
    /// The frame is on screen.
    Shown,
    /// The frame is on screen, but the 3D render hung, or a copy between
    /// the screens stalled the 2D engine, and was recovered: the area is
    /// stale until the next frame (which redraws it all). Each stalled copy
    /// is counted ([`Graphics::copy_stalls`]).
    Recovered,
}

/// How long to wait before looking at the 2D engine again (it has no
/// prediction; a full-screen 2D frame takes milliseconds).
const TWO_D_POLL_US: u32 = 200;
/// A copy this many pixels or more is timed into the copies' schedule
/// (an eighth of the screen: smaller ones are mostly the blit's own cost,
/// which would overstate the per-pixel time a big copy is planned with).
const TIMED_COPY_PX: u32 = scan::SCREEN_PX / 8;

/// The service's 3D state: the scene and the texture blocks (about 5 KiB).
/// Kept by the program and lent to the service only by programs that use
/// 3D, so a 2D-only one (the Today screen) doesn't carry it (stack audit,
/// 2026-10-17: it was 5 of the 5.4 KiB of `Graphics`, all unused there).
pub struct ThreeD {
    scene: Option<Scene>,
    blocks: [Block; BLOCKS],
}

impl ThreeD {
    /// No scene, no textures.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scene: None,
            blocks: [NO_BLOCK; BLOCKS],
        }
    }
}

impl Default for ThreeD {
    fn default() -> Self {
        Self::new()
    }
}

/// The owner of the 2D engine and the 3D core. It borrows the 2D renderer
/// (5-6 KiB) and the 3D state rather than holding them: moving one into the
/// other put both on a test image's stack at once and overflowed it
/// (`rp-today` in the emulator, 2026-10-15); a test keeps this struct small.
pub struct Graphics<'a, R, M> {
    two_d: &'a mut Mbx2dRenderer<R, M>,
    three_d: Option<&'a mut ThreeD>,
    pool: Pool,
    clients: [ClientRec; CLIENTS],
    job: Job,
    copy_stalls: u32,
    /// R20: the scene's recent render times.
    sched: Schedule,
    /// R20: recent copy times between the screens, per whole screen.
    copies: Schedule,
    /// R20's fallback into screen 1 allowed (screen 1's soak passed).
    screen1_3d: bool,
    starts: Starts,
    /// Tests: the copy (1 = the next) before which the hook runs (it makes
    /// the model engine stall).
    #[cfg(test)]
    before_copy: Option<(u8, CopyHook<R, M>)>,
}

impl<'a, R: RegisterBus, M: LocalMem> Graphics<'a, R, M> {
    /// The service over the 2D renderer (which holds the register window
    /// and memory, its flips held for the 3D area), the pool, and the 3D
    /// state if the program uses 3D (without it, 3D requests are refused
    /// with [`Error::No3d`] and frames are 2D only).
    pub fn new(
        two_d: &'a mut Mbx2dRenderer<R, M>,
        pool: Pool,
        three_d: Option<&'a mut ThreeD>,
    ) -> Self {
        two_d.hold = true;
        Self {
            two_d,
            three_d,
            pool,
            clients: [ClientRec::default(); CLIENTS],
            job: Job::None,
            copy_stalls: 0,
            sched: Schedule::new(),
            copies: Schedule::new(),
            screen1_3d: false,
            starts: Starts {
                behind: 0,
                after_area: 0,
                screen1: 0,
                last_line: None,
                last_copy_line: None,
            },
            #[cfg(test)]
            before_copy: None,
        }
    }

    /// Copies between the screens that stalled the 2D engine (C71 ruling
    /// R-B's two copies): each frame it hit was reported
    /// [`Step::Recovered`].
    #[must_use]
    pub const fn copy_stalls(&self) -> u32 {
        self.copy_stalls
    }

    /// R20's fallback into screen 1 on or off: scene frames the scan can't
    /// lead render into screen 1 (outside the safe region) while screen 0
    /// is shown, then are copied into screen 0 behind the scan. Off until
    /// the soak of 3D targets at screen 1's place (`rp-gpubench` GC2DE) has
    /// passed on the X51v at the clock in use (ADR-0035); off, they start
    /// on screen 0 once the scan has passed the scene's bottom row.
    pub fn allow_screen1_3d(&mut self, on: bool) {
        self.screen1_3d = on;
    }

    /// How the 3D frames started so far (R20).
    #[must_use]
    pub const fn starts(&self) -> Starts {
        self.starts
    }

    /// The display's scan from its pixel clock (R111: each measured on the
    /// X51v), or `None` for a clock without a measured scan: no frame then
    /// starts behind the scan.
    fn scan(&mut self) -> Option<Scan> {
        match self.two_d.parts().0.regs.read32(dreg::DISPPLL) {
            pll::SIXTY => Some(scan::X51V_60HZ),
            pll::WM => Some(scan::X51V),
            _ => None,
        }
    }

    /// The scan line now.
    fn scan_line(&mut self) -> u32 {
        self.two_d.parts().0.regs.read32(dreg::DVLNUM) & 0xFFF
    }

    /// When the scan reaches `line` (the next time), on the clock.
    fn when_line<C: Clock>(&mut self, line: u32, clock: &mut C) -> u32 {
        let total = scan::X51V_60HZ.total;
        let ns = self.scan().unwrap_or(scan::X51V_60HZ).line_ns;
        let lines = (line % total + total - self.scan_line() % total) % total;
        clock
            .now_us()
            .wrapping_add((u64::from(lines.max(1)) * u64::from(ns) / 1000) as u32)
    }

    /// How the open scene's next frame starts (R20), recomputed every
    /// frame: behind the scan where the schedule fits it; else into screen
    /// 1 (`aside_ok`: screen 1 holds the frame, and the fallback into it is
    /// allowed) when the copy of the area and of `held` (2D's changes held
    /// in screen 1) has a window; else on screen 0 once the scan has passed
    /// the scene's bottom row, by the refresh's last line.
    fn plan_3d(&mut self, aside_ok: bool, held: &[Rect]) -> Option<Start> {
        let scan = self.scan();
        let s = self.open()?;
        let band = (u32::from(s.y), u32::from(s.h));
        let area = Rect::new(s.x, s.y, s.w, s.h);
        let top_down = s.r.tile_order().top_down();
        if let Some((from, to)) = scan.and_then(|sc| self.sched.window(&sc, top_down, band)) {
            return Some(Start::Screen0 {
                from,
                to,
                behind: true,
            });
        }
        if aside_ok && self.screen1_3d {
            let mut rects = [Rect::default(); MAX_DIRTY + 1];
            let n = copy_rects(&mut rects, area, held);
            if let Some((from, to)) = self.copy_window(&rects[..n]) {
                return Some(Start::Screen1 { from, to });
            }
        }
        let from = band.0 + band.1;
        let last = scan.unwrap_or(scan::X51V_60HZ).total - 1;
        Some(Start::Screen0 {
            from,
            to: (from + AFTER_AREA_LINES).min(last).max(from),
            behind: false,
        })
    }

    /// The window a copy of `rects` (in order) may start in on the
    /// display's scan, from the copies' recent times (`None`: no copy
    /// timed yet, an unmeasured clock, or no window).
    fn copy_window(&mut self, rects: &[Rect]) -> Option<(u32, u32)> {
        let scan = self.scan()?;
        let band = self.copies.band()?;
        let mut spans = [(0, 0, 0); MAX_DIRTY + 1];
        for (sp, r) in spans.iter_mut().zip(rects) {
            *sp = (
                u32::from(r.y),
                u32::from(r.h),
                u32::from(r.w) * u32::from(r.h),
            );
        }
        scan::copy_window(&scan, band, &spans[..rects.len()])
    }

    /// The scene's target: its area of the picture at `at`.
    fn target(s: &Scene, at: usize) -> Target {
        Target {
            picture: Picture {
                at: at as u32,
                ..Picture::screen(0)
            },
            x: s.x,
            y: s.y,
        }
    }

    /// [`Mbx2dRenderer::copy_screens`], counted when it stalls; one of an
    /// eighth of the screen or more is timed into the copies' schedule.
    fn copy_screens<C: Clock>(
        &mut self,
        dst: usize,
        src: usize,
        rects: &[Rect],
        clock: &mut C,
    ) -> bool {
        let t0 = clock.now_us();
        #[cfg(test)]
        if let Some((n, hook)) = self.before_copy {
            self.before_copy = if n > 1 { Some((n - 1, hook)) } else { None };
            if n == 1 {
                hook(self.two_d);
            }
        }
        let ok = self.two_d.copy_screens(dst, src, rects);
        let us = clock.now_us().wrapping_sub(t0);
        let px: u32 = rects.iter().map(|r| u32::from(r.w) * u32::from(r.h)).sum();
        if !ok {
            self.copy_stalls += 1;
        } else if px >= TIMED_COPY_PX && us > 0 {
            let whole = u64::from(us) * u64::from(scan::SCREEN_PX) / u64::from(px);
            self.copies.record(u32::try_from(whole).unwrap_or(u32::MAX));
        }
        ok
    }

    /// The open scene, if any.
    fn open(&self) -> Option<&Scene> {
        self.three_d.as_deref()?.scene.as_ref()
    }

    /// The open scene, if any, to change.
    fn open_mut(&mut self) -> Option<&mut Scene> {
        self.three_d.as_deref_mut()?.scene.as_mut()
    }

    /// The 2D renderer (wallpaper, showing the picture: the service's own
    /// set-up).
    pub fn two_d(&mut self) -> &mut Mbx2dRenderer<R, M> {
        self.two_d
    }

    /// A new client allowed `quota` bytes of 2700G memory.
    pub fn client(&mut self, quota: u32) -> Option<ClientId> {
        let i = self.clients.iter().position(|c| !c.live)?;
        self.clients[i] = ClientRec {
            live: true,
            quota,
            used: 0,
        };
        Some(ClientId(i as u8))
    }

    fn rec(&self, c: ClientId) -> Result<ClientRec, Error> {
        self.clients
            .get(usize::from(c.0))
            .copied()
            .filter(|r| r.live)
            .ok_or(Error::NoClient)
    }

    /// Bytes of 2700G memory the client holds.
    pub fn used(&self, c: ClientId) -> u32 {
        self.rec(c).map_or(0, |r| r.used)
    }

    /// Ends a client (with nothing in flight): its blocks go back to the
    /// pool, its scene closes.
    pub fn close(&mut self, c: ClientId) -> Result<(), Error> {
        self.rec(c)?;
        if self.open().is_some_and(|s| s.owner == c.0) {
            self.close_scene(c)?;
        }
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        for k in 0..BLOCKS {
            if self
                .three_d
                .as_deref()
                .is_some_and(|t| t.blocks[k].tex.is_some() && t.blocks[k].owner == c.0)
            {
                self.drop_block(k);
            }
        }
        self.clients[usize::from(c.0)] = ClientRec::default();
        Ok(())
    }

    fn drop_block(&mut self, k: usize) {
        let Some(three_d) = self.three_d.as_deref_mut() else {
            return;
        };
        let b = &mut three_d.blocks[k];
        if let Some(t) = b.tex.take() {
            // A false here is the service's own bookkeeping gone wrong (a
            // block freed twice), never a run-time condition.
            let freed = r3d::release(&mut self.pool, t);
            debug_assert!(freed, "texture block not in the pool");
            let owner = usize::from(b.owner);
            self.clients[owner].used = self.clients[owner].used.saturating_sub(t.bytes());
            b.generation = b.generation.wrapping_add(1);
        }
    }

    /// Uploads a texture of 16-bit texels for client `c` (see
    /// `rp_mbx3d_render::upload`), within its quota.
    pub fn upload_texture(
        &mut self,
        c: ClientId,
        code: u8,
        format: u32,
        texels: &[u16],
    ) -> Result<Handle, Error> {
        self.upload_with(c, code, format, 1, |pool, mem| {
            r3d::upload(pool, mem, code, format, texels)
        })
    }

    /// Uploads a texture of 16-bit texels with its mip levels made on the
    /// CPU (see `rp_mbx3d_render::upload_mipmapped`) for client `c`: a
    /// third more against its quota.
    pub fn upload_mipmapped(
        &mut self,
        c: ClientId,
        code: u8,
        format: u32,
        texels: &[u16],
    ) -> Result<Handle, Error> {
        self.upload_with(c, code, format, code.min(7) + 4, |pool, mem| {
            r3d::upload_mipmapped(pool, mem, code, format, texels)
        })
    }

    /// Uploads a texture of 32-bit texels (`0xAARRGGBB`) for client `c`.
    pub fn upload_texture32(
        &mut self,
        c: ClientId,
        code: u8,
        texels: &[u32],
    ) -> Result<Handle, Error> {
        self.upload_with(c, code, r3d::format::ARGB8888, 1, |pool, mem| {
            r3d::upload32(pool, mem, code, texels)
        })
    }

    /// Uploads a PVRTC texture compressed on the PC (see
    /// `rp_mbx3d_render::upload_pvrtc`) for client `c`: 4 or 2 bits a
    /// texel against its quota.
    pub fn upload_pvrtc(
        &mut self,
        c: ClientId,
        code: u8,
        format: u32,
        blocks: &[u32],
    ) -> Result<Handle, Error> {
        self.upload_with(c, code, format, 1, |pool, mem| {
            r3d::upload_pvrtc(pool, mem, code, format, blocks)
        })
    }

    /// A texture uploaded by `up` for client `c`, checked against its
    /// quota first (by the format's size).
    fn upload_with(
        &mut self,
        c: ClientId,
        code: u8,
        format: u32,
        levels: u8,
        up: impl FnOnce(&mut Pool, &mut M) -> Option<Texture>,
    ) -> Result<Handle, Error> {
        let rec = self.rec(c)?;
        let bytes = Texture {
            at: 0,
            code: code.min(7),
            format: format as u8,
            u_mode: 0,
            v_mode: 0,
            linear: false,
            levels,
        }
        .bytes();
        let three_d = self.three_d.as_deref_mut().ok_or(Error::No3d)?;
        if rec.used.saturating_add(bytes) > rec.quota {
            return Err(Error::Quota);
        }
        let k = three_d
            .blocks
            .iter()
            .position(|b| b.tex.is_none())
            .ok_or(Error::NoMemory)?;
        let (_, mem) = self.two_d.parts();
        let t = up(&mut self.pool, mem).ok_or(Error::NoMemory)?;
        let b = &mut three_d.blocks[k];
        b.owner = c.0;
        b.tex = Some(t);
        self.clients[usize::from(c.0)].used += t.bytes();
        Ok(Handle {
            slot: k as u16,
            generation: b.generation,
        })
    }

    /// Client `c`'s texture behind `h`.
    fn texture(&self, c: ClientId, h: Handle) -> Result<Texture, Error> {
        let b = self
            .three_d
            .as_deref()
            .ok_or(Error::No3d)?
            .blocks
            .get(usize::from(h.slot))
            .ok_or(Error::BadHandle)?;
        match b.tex {
            Some(t) if b.owner == c.0 && b.generation == h.generation => Ok(t),
            _ => Err(Error::BadHandle),
        }
    }

    /// Gives a texture back (with nothing in flight). The caller empties
    /// the quads using it first.
    pub fn free(&mut self, c: ClientId, h: Handle) -> Result<(), Error> {
        self.rec(c)?;
        self.texture(c, h)?;
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        self.drop_block(usize::from(h.slot));
        Ok(())
    }

    /// Opens the 3D scene for client `c`: the area `w` x `h` at `(x, y)`
    /// (whole 8 x 16 tiles) belongs to 3D from now on (G1, option A):
    /// whatever 2D draws there is drawn over by the scene before each
    /// flip.
    pub fn open_scene(&mut self, c: ClientId, x: u16, y: u16, w: u16, h: u16) -> Result<(), Error> {
        self.rec(c)?;
        if self.three_d.is_none() {
            return Err(Error::No3d);
        }
        if self.open().is_some() || self.job != Job::None {
            return Err(Error::Busy);
        }
        let pic = Picture::screen(0);
        if u32::from(x) + u32::from(w) > u32::from(pic.w)
            || u32::from(y) + u32::from(h) > u32::from(pic.h)
        {
            return Err(Error::Scene(r3d::Error::BadTarget));
        }
        let r = r3d::Renderer::new(&mut self.pool, w, h).ok_or(Error::NoMemory)?;
        if let Some(t) = self.three_d.as_deref_mut() {
            t.scene = Some(Scene {
                owner: c.0,
                r,
                x,
                y,
                w,
                h,
                resets: 0,
            });
        }
        Ok(())
    }

    /// Closes client `c`'s scene (with nothing in flight); its area goes
    /// back to 2D, redrawn whole on the next frame.
    pub fn close_scene(&mut self, c: ClientId) -> Result<(), Error> {
        if !self.open().is_some_and(|s| s.owner == c.0) {
            return Err(Error::NotYourScene);
        }
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        if let Some(s) = self.three_d.as_deref_mut().and_then(|t| t.scene.take()) {
            let freed = s.r.release(&mut self.pool);
            debug_assert!(freed, "the scene's regions not in the pool");
        }
        self.two_d.invalidate_all();
        Ok(())
    }

    fn scene(&mut self, c: ClientId) -> Result<&mut Scene, Error> {
        match self.open_mut() {
            Some(s) if s.owner == c.0 => Ok(s),
            _ => Err(Error::NotYourScene),
        }
    }

    /// Client `c`'s scene background (see `Renderer::set_background`).
    pub fn set_background(&mut self, c: ClientId, argb: Option<u32>) -> Result<(), Error> {
        self.scene(c)?.r.set_background(argb).map_err(Error::Scene)
    }

    /// Puts `q` in slot `slot` of client `c`'s scene (or empties it); its
    /// texture must be one of `c`'s. Positions are the area's.
    pub fn set(&mut self, c: ClientId, slot: usize, q: Option<&Quad>) -> Result<(), Error> {
        let draw = match q {
            None => None,
            Some(q) => {
                let paint = match q.fill {
                    Fill::Flat(c) => Paint::Flat(c),
                    Fill::Gouraud(cs) => Paint::Gouraud(cs),
                    Fill::Texture {
                        tex,
                        uv,
                        tint,
                        u_mode,
                        v_mode,
                        linear,
                    } => Paint::Texture {
                        tex: self
                            .texture(c, tex)?
                            .with_modes(u_mode, v_mode)
                            .with_linear(linear),
                        uv,
                        tint,
                    },
                };
                Some(Draw {
                    corners: q.corners,
                    paint,
                    blend: q.blend,
                    depth: q.depth,
                    alpha: q.alpha,
                })
            }
        };
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        let s = match self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) {
            Some(s) if s.owner == c.0 => s,
            _ => return Err(Error::NotYourScene),
        };
        let (_, mem) = self.two_d.parts();
        s.r.set(mem, slot, draw.as_ref()).map_err(Error::Scene)
    }

    /// Starts a frame: the 2D `frame`, then the 3D area over it in the
    /// same picture, then the flip ([`Self::poll`] drives it). A frame with
    /// no 2D change still shows the scene's changes.
    pub fn frame<C: Clock>(&mut self, frame: &Frame, clock: &mut C) -> Result<Step, Error> {
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        self.two_d.draw(frame);
        if self.two_d.landed().is_some() || !self.two_d.poll() {
            self.job = Job::TwoD;
            return Ok(self.poll(clock));
        }
        // No 2D change. Nothing changed in the scene either: nothing to do.
        if self.open().is_none_or(|s| s.r.dirty() == 0) {
            return Ok(Step::Idle);
        }
        // The 3D core draws only with screen 0 on screen (C71 ruling R-B).
        // If screen 1 is, both hold the same frame (`back`): screen 0 is
        // shown first, a whole frame.
        let [safe, other] = self.two_d.screens();
        let flipped = self.two_d.front() != safe;
        if flipped {
            if self.two_d.back() != Some(safe) {
                return Ok(Step::Idle); // screen 0 not caught up yet
            }
            self.two_d.flip_back();
            self.two_d.settle();
        }
        // Screen 1 holds the same frame as screen 0 (the fallback may draw
        // it).
        let same = self.two_d.back() == Some(other);
        let step = match self.plan_3d(same, &[]) {
            Some(Start::Screen1 { from, to }) => self.aside(other, false, (from, to), clock),
            Some(start) => self.arm(start, clock),
            None => Step::Idle,
        };
        Ok(match step {
            Step::Idle if flipped => Step::Shown,
            s => s,
        })
    }

    /// Readies the scene's frame to start on screen 0 (on screen) at its
    /// window's line ([`Job::Armed`]), its lists written now where the core
    /// allows (so the start is only the start); `Idle` if nothing changed.
    fn arm<C: Clock>(&mut self, start: Start, clock: &mut C) -> Step {
        let Start::Screen0 { from, to, behind } = start else {
            return Step::Idle;
        };
        let safe = self.two_d.screens()[0];
        let Some(s) = self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) else {
            return Step::Idle;
        };
        let target = Self::target(s, safe);
        let (e, mem) = self.two_d.parts();
        let prepared = match s.r.prepare(&mut e.regs, mem, clock, target) {
            Ok(true) => true,
            Ok(false) => return Step::Idle,
            // The core needs setting up: written at the start instead.
            Err(r3d::Error::Busy) => false,
            Err(_) => return Step::Idle,
        };
        self.job = Job::Armed {
            from,
            to,
            behind,
            prepared,
        };
        self.poll(clock)
    }

    /// Starts the scene's frame into screen 1 at `other` while screen 0 is
    /// shown (R20's fallback), its copy into screen 0 to start at a line in
    /// `from..=to`: `held`, 2D's frame is held there.
    fn aside<C: Clock>(
        &mut self,
        other: usize,
        held: bool,
        (from, to): (u32, u32),
        clock: &mut C,
    ) -> Step {
        let Some(s) = self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) else {
            return Step::Idle;
        };
        let target = Self::target(s, other);
        let (e, mem) = self.two_d.parts();
        let res = s.r.render(&mut e.regs, mem, clock, target);
        self.after_3d();
        match res {
            Ok(Some(due)) => {
                self.starts.screen1 += 1;
                self.job = Job::Aside { held, from, to };
                Step::Wait(due)
            }
            _ => {
                if held && !self.give_held(clock) {
                    return Step::Recovered;
                }
                Step::Idle
            }
        }
    }

    /// 2D's frame held in screen 1 reaches screen 0 (on screen): its
    /// changes copied in, the held frame dropped (both screens then hold
    /// it); if the copy stalls, screen 1 is shown instead (whole; the 3D
    /// core is idle: amendment P rule 4), `false`.
    fn give_held<C: Clock>(&mut self, clock: &mut C) -> bool {
        let mut drawn = [Rect::default(); MAX_DIRTY];
        let n = self.two_d.drawn().len();
        drawn[..n].copy_from_slice(self.two_d.drawn());
        let [safe, other] = self.two_d.screens();
        if self.copy_screens(safe, other, &drawn[..n], clock) {
            self.two_d.keep_front();
            true
        } else {
            self.two_d.flip();
            false
        }
    }

    /// A render seen done (its time into the schedule) or hung (the
    /// schedule forgets: the next frame redraws everything).
    fn learn(&mut self, p: Poll) {
        match p {
            Poll::Done(st) => self.sched.record(st.done_us),
            Poll::Recovered => self.sched.reset(),
            Poll::Wait(_) | Poll::Idle => {}
        }
    }

    /// Drives the frame in flight.
    pub fn poll<C: Clock>(&mut self, clock: &mut C) -> Step {
        match self.job {
            Job::None => Step::Idle,
            Job::TwoD => {
                let Some(at) = self.two_d.landed() else {
                    let _ = self.two_d.poll();
                    return if self.two_d.landed().is_some() {
                        self.poll(clock)
                    } else {
                        Step::Wait(clock.now_us().wrapping_add(TWO_D_POLL_US))
                    };
                };
                // What 2D drew afresh over the area: the scene draws it
                // again.
                let mut drawn = [Rect::default(); MAX_DIRTY];
                let n = self.two_d.drawn().len();
                drawn[..n].copy_from_slice(self.two_d.drawn());
                let needs_3d = match self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) {
                    Some(s) => {
                        for r in &drawn[..n] {
                            s.r.invalidate(
                                i32::from(r.x) - i32::from(s.x),
                                i32::from(r.y) - i32::from(s.y),
                                i32::from(r.w),
                                i32::from(r.h),
                            );
                        }
                        s.r.dirty() != 0
                    }
                    None => false,
                };
                self.job = Job::None;
                if !needs_3d {
                    // A 2D frame: flipped as ever, to either screen (the 3D
                    // core is idle: amendment P rule 4).
                    self.two_d.flip();
                    return Step::Shown;
                }
                // The 3D core draws only with screen 0 on screen (C71 ruling
                // R-B: screen 1 lies outside the safe region).
                let [safe, other] = self.two_d.screens();
                // 2D's frame held in screen 1 (screen 0 shown): its changes
                // go into screen 0 with the scene's area.
                let held: &[Rect] = if at == safe { &[] } else { &drawn[..n] };
                let start = self.plan_3d(true, held);
                if let Some(Start::Screen1 { from, to }) = start {
                    if at != safe {
                        // The scene renders into screen 1 with 2D's frame,
                        // and both are copied into screen 0.
                        return self.aside(other, true, (from, to), clock);
                    }
                    // 2D drew into screen 0: it is shown first, then its
                    // changes go into screen 1 for the scene.
                    self.two_d.flip();
                    self.two_d.settle();
                    if !self.copy_screens(other, safe, &drawn[..n], clock) {
                        return Step::Recovered;
                    }
                    return self.aside(other, false, (from, to), clock);
                }
                if at == safe {
                    // 2D's frame shown whole, scan-out off screen 1.
                    self.two_d.flip();
                    self.two_d.settle();
                } else if self.copy_screens(safe, at, &drawn[..n], clock) {
                    // Screen 0 is on screen: 2D's changes copied into it
                    // (seen as they land, as the 3D area is: R20 covers
                    // neither yet), the held frame dropped.
                    self.two_d.keep_front();
                } else {
                    // The copy stalled, leaving screen 0 part-changed: the
                    // held frame shown instead (whole, its area the last
                    // render's), the render left to the next frame, which
                    // the stall draws whole (so the scene renders again).
                    // The 3D core is idle, so screen 1 may be shown
                    // (amendment P rule 4).
                    self.two_d.flip();
                    return Step::Recovered;
                }
                match start.map(|st| self.arm(st, clock)) {
                    Some(Step::Idle) | None => Step::Shown,
                    Some(step) => step,
                }
            }
            Job::Armed {
                from,
                to,
                behind,
                prepared,
            } => {
                let total = scan::X51V_60HZ.total;
                let line = self.scan_line();
                let inside = if to < total {
                    (from..=to).contains(&line)
                } else {
                    line >= from || line <= to - total
                };
                if !inside {
                    return Step::Wait(self.when_line(from, clock));
                }
                let safe = self.two_d.screens()[0];
                let Some(s) = self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) else {
                    self.job = Job::None;
                    return Step::Shown;
                };
                let target = Self::target(s, safe);
                let (e, mem) = self.two_d.parts();
                let due = if prepared {
                    s.r.start_prepared(&mut e.regs, clock)
                } else {
                    s.r.render(&mut e.regs, mem, clock, target)
                };
                self.after_3d();
                self.starts.last_line = Some(line);
                if behind {
                    self.starts.behind += 1;
                } else {
                    self.starts.after_area += 1;
                }
                match due {
                    Ok(Some(due)) => {
                        self.job = Job::ThreeD;
                        Step::Wait(due)
                    }
                    _ => {
                        self.job = Job::None;
                        Step::Shown
                    }
                }
            }
            Job::Aside { held, from, to } => {
                let p = match self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) {
                    Some(s) => {
                        let (e, mem) = self.two_d.parts();
                        s.r.poll(&mut e.regs, mem, clock)
                    }
                    None => Poll::Idle,
                };
                self.after_3d();
                self.learn(p);
                match p {
                    Poll::Wait(t) => return Step::Wait(t),
                    Poll::Recovered => {
                        // Screen 0 stays on screen, whole; held, 2D's frame
                        // goes into it instead (the area stale until the
                        // next frame, which the hang draws whole).
                        self.job = Job::None;
                        if held {
                            self.give_held(clock);
                        }
                        return Step::Recovered;
                    }
                    Poll::Done(_) | Poll::Idle => {}
                }
                // Rendered: the copy into screen 0 waits for its window.
                self.job = Job::Copy { held, from, to };
                self.poll(clock)
            }
            Job::Copy { held, from, to } => {
                let line = self.scan_line();
                if !(from..=to).contains(&line) {
                    return Step::Wait(self.when_line(from, clock));
                }
                let Some(area) = self.open().map(|s| Rect::new(s.x, s.y, s.w, s.h)) else {
                    self.job = Job::None;
                    return Step::Shown;
                };
                // Screen 0 on screen: the area (and 2D's changes, held)
                // copied in behind the scan, in the planned order; both
                // screens then hold the frame.
                let mut drawn = [Rect::default(); MAX_DIRTY];
                let n = if held { self.two_d.drawn().len() } else { 0 };
                drawn[..n].copy_from_slice(&self.two_d.drawn()[..n]);
                let mut rects = [Rect::default(); MAX_DIRTY + 1];
                let k = copy_rects(&mut rects, area, &drawn[..n]);
                let [safe, other] = self.two_d.screens();
                self.job = Job::None;
                self.starts.last_copy_line = Some(line);
                if self.copy_screens(safe, other, &rects[..k], clock) {
                    if held {
                        self.two_d.keep_front();
                    }
                    Step::Shown
                } else {
                    // Screen 0 part-copied: screen 1, whole, is shown
                    // instead (the 3D core idle: amendment P rule 4); the
                    // stall draws the next frame whole.
                    if held {
                        self.two_d.flip();
                    } else {
                        self.two_d.flip_back();
                    }
                    Step::Recovered
                }
            }
            Job::ThreeD => {
                let (p, area) = match self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) {
                    Some(s) => {
                        let (e, mem) = self.two_d.parts();
                        (
                            s.r.poll(&mut e.regs, mem, clock),
                            Some(Rect::new(s.x, s.y, s.w, s.h)),
                        )
                    }
                    None => (Poll::Idle, None),
                };
                self.after_3d();
                self.learn(p);
                let mut shown = match p {
                    Poll::Wait(t) => return Step::Wait(t),
                    Poll::Recovered => Step::Recovered,
                    Poll::Done(_) | Poll::Idle => Step::Shown,
                };
                // The area carried into screen 1, so a later 2D frame
                // flipped there shows it current. If the copy stalls, the
                // frame on screen is still whole, but screen 1's area is
                // old: the stall draws the next frame whole, which renders
                // the scene again before anything is shown from screen 1.
                if let Some(area) = area {
                    let [safe, other] = self.two_d.screens();
                    if !self.copy_screens(other, safe, &[area], clock) {
                        shown = Step::Recovered;
                    }
                }
                self.job = Job::None;
                shown
            }
        }
    }

    /// The 2D engine's stream restarts after a reset the 3D renderer made
    /// (the reset is the whole MBX's).
    fn after_3d(&mut self) {
        let Some(s) = self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) else {
            return;
        };
        if s.r.core_resets() != s.resets {
            s.resets = s.r.core_resets();
            self.two_d.parts().0.after_core_reset();
        }
    }

    /// When idle at `now_us`: the 3D clock off once unused for a while.
    pub fn idle(&mut self, now_us: u32) -> bool {
        if self.job != Job::None {
            return false;
        }
        let Some(s) = self.three_d.as_deref_mut().and_then(|t| t.scene.as_mut()) else {
            return false;
        };
        let (e, _) = self.two_d.parts();
        s.r.idle(&mut e.regs, now_us)
    }
}

/// The fallback's copy (R20): the scene's `area` and 2D's `held` changes,
/// top first (each copied top down, so the whole copy runs down the
/// screen), any one inside another left out (a whole-screen 2D frame
/// holds the area: copied twice, the area's second copy would land after
/// the scan came round), into `out`; how many.
fn copy_rects(out: &mut [Rect; MAX_DIRTY + 1], area: Rect, held: &[Rect]) -> usize {
    let held = &held[..held.len().min(MAX_DIRTY)];
    let mut n = 0;
    for (i, &r) in held.iter().chain([area].iter()).enumerate() {
        let inside = |o: &Rect| {
            o.x <= r.x
                && o.y <= r.y
                && u32::from(o.x) + u32::from(o.w) >= u32::from(r.x) + u32::from(r.w)
                && u32::from(o.y) + u32::from(o.h) >= u32::from(r.y) + u32::from(r.h)
        };
        // Inside an earlier one, or strictly inside a later one.
        let covered = held
            .iter()
            .chain([area].iter())
            .enumerate()
            .any(|(j, o)| j != i && inside(o) && (j < i || *o != r));
        if !covered && r.w != 0 && r.h != 0 {
            out[n] = r;
            n += 1;
        }
    }
    out[..n].sort_unstable_by_key(|r| (r.y, r.x));
    n
}

#[cfg(test)]
mod tests;
