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
//!   owner's vote): each frame, 2D draws into the picture not on screen,
//!   the scene renders its area into the same picture (the tiles 2D drew
//!   over included), then the picture is shown; a frame with only 3D
//!   changes renders into the other picture (it holds the same 2D frame)
//!   and shows it. Nothing half-drawn or stale reaches the screen.
//!
//! A library now (G4), used in one process; the display driver host's once
//! IPC can move pages (ADR-0016).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_draw::{Frame, Renderer as _};
use rp_hal_api::RegisterBus;
use rp_mbx_render::{LocalMem, Mbx2dRenderer};
use rp_mbx3d_render::{
    self as r3d, Clock, Depth, Draw, Paint, Picture, Point, Poll, Target, Texture,
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
    /// Its renderer's core resets already passed on to the 2D engine.
    resets: u32,
}

/// The job in flight (one at a time, G2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    None,
    /// A 2D frame, its flip held for the 3D area.
    TwoD,
    /// The 3D area rendering; then the held frame (or the back picture)
    /// is shown.
    ThreeD {
        back: bool,
    },
}

/// What [`Graphics::poll`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing in flight.
    Idle,
    /// Not done: poll again at this time (microseconds, the clock's).
    Wait(u32),
    /// The frame is on screen.
    Shown,
    /// The frame is on screen, but the 3D render hung and was recovered:
    /// its area is stale until the next frame (which redraws it all).
    Recovered,
}

/// How long to wait before looking at the 2D engine again (it has no
/// prediction; a full-screen 2D frame takes milliseconds).
const TWO_D_POLL_US: u32 = 200;

/// The owner of the 2D engine and the 3D core. It borrows the 2D renderer
/// (5-6 KiB) rather than holding it: moving one into the other put both on
/// a test image's stack at once and overflowed it (`rp-today` in the
/// emulator, 2026-10-15); a test keeps this struct small.
pub struct Graphics<'a, R, M> {
    two_d: &'a mut Mbx2dRenderer<R, M>,
    scene: Option<Scene>,
    pool: Pool,
    clients: [ClientRec; CLIENTS],
    blocks: [Block; BLOCKS],
    job: Job,
}

impl<'a, R: RegisterBus, M: LocalMem> Graphics<'a, R, M> {
    /// The service over the 2D renderer (which holds the register window
    /// and memory, its flips held for the 3D area) and the pool.
    pub fn new(two_d: &'a mut Mbx2dRenderer<R, M>, pool: Pool) -> Self {
        two_d.hold = true;
        Self {
            two_d,
            scene: None,
            pool,
            clients: [ClientRec::default(); CLIENTS],
            blocks: [NO_BLOCK; BLOCKS],
            job: Job::None,
        }
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
        if self.scene.as_ref().is_some_and(|s| s.owner == c.0) {
            self.close_scene(c)?;
        }
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        for k in 0..BLOCKS {
            if self.blocks[k].tex.is_some() && self.blocks[k].owner == c.0 {
                self.drop_block(k);
            }
        }
        self.clients[usize::from(c.0)] = ClientRec::default();
        Ok(())
    }

    fn drop_block(&mut self, k: usize) {
        if let Some(t) = self.blocks[k].tex.take() {
            let _ = r3d::release(&mut self.pool, t);
            let owner = usize::from(self.blocks[k].owner);
            self.clients[owner].used = self.clients[owner].used.saturating_sub(t.bytes());
            self.blocks[k].generation = self.blocks[k].generation.wrapping_add(1);
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
        if rec.used.saturating_add(bytes) > rec.quota {
            return Err(Error::Quota);
        }
        let k = self
            .blocks
            .iter()
            .position(|b| b.tex.is_none())
            .ok_or(Error::NoMemory)?;
        let (_, mem) = self.two_d.parts();
        let t = up(&mut self.pool, mem).ok_or(Error::NoMemory)?;
        self.blocks[k].owner = c.0;
        self.blocks[k].tex = Some(t);
        self.clients[usize::from(c.0)].used += t.bytes();
        Ok(Handle {
            slot: k as u16,
            generation: self.blocks[k].generation,
        })
    }

    /// Client `c`'s texture behind `h`.
    fn texture(&self, c: ClientId, h: Handle) -> Result<Texture, Error> {
        let b = self
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
        if self.scene.is_some() || self.job != Job::None {
            return Err(Error::Busy);
        }
        let pic = Picture::screen(0);
        if u32::from(x) + u32::from(w) > u32::from(pic.w)
            || u32::from(y) + u32::from(h) > u32::from(pic.h)
        {
            return Err(Error::Scene(r3d::Error::BadTarget));
        }
        let r = r3d::Renderer::new(&mut self.pool, w, h).ok_or(Error::NoMemory)?;
        self.scene = Some(Scene {
            owner: c.0,
            r,
            x,
            y,
            resets: 0,
        });
        Ok(())
    }

    /// Closes client `c`'s scene (with nothing in flight); its area goes
    /// back to 2D, redrawn whole on the next frame.
    pub fn close_scene(&mut self, c: ClientId) -> Result<(), Error> {
        if !self.scene.as_ref().is_some_and(|s| s.owner == c.0) {
            return Err(Error::NotYourScene);
        }
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        if let Some(s) = self.scene.take() {
            let _ = s.r.release(&mut self.pool);
        }
        self.two_d.invalidate_all();
        Ok(())
    }

    fn scene(&mut self, c: ClientId) -> Result<&mut Scene, Error> {
        match &mut self.scene {
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
                })
            }
        };
        if self.job != Job::None {
            return Err(Error::Busy);
        }
        let s = match &mut self.scene {
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
        // No 2D change: both pictures hold the same 2D frame, so the scene
        // renders into the one not on screen, which is then shown. Nothing
        // changed in the scene either: nothing to do (the picture not on
        // screen catches up on the next frame drawn into it).
        if self.scene.as_ref().is_none_or(|s| s.r.dirty() == 0) {
            return Ok(Step::Idle);
        }
        let Some(back) = self.two_d.back() else {
            return Ok(Step::Idle);
        };
        match self.start_3d(back, clock) {
            Some(due) => {
                self.job = Job::ThreeD { back: true };
                Ok(Step::Wait(due))
            }
            None => Ok(Step::Idle),
        }
    }

    /// Renders the scene into its area of the picture at `at`: when to
    /// poll, or `None` if there's nothing to render (or no scene).
    fn start_3d<C: Clock>(&mut self, at: usize, clock: &mut C) -> Option<u32> {
        let s = self.scene.as_mut()?;
        let target = Target {
            picture: Picture {
                at: at as u32,
                ..Picture::screen(0)
            },
            x: s.x,
            y: s.y,
        };
        let (e, mem) = self.two_d.parts();
        let res = s.r.render(&mut e.regs, mem, clock, target);
        self.after_3d();
        res.ok().flatten()
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
                // again before the flip.
                if let Some(s) = &mut self.scene {
                    for r in self.two_d.drawn() {
                        s.r.invalidate(
                            i32::from(r.x) - i32::from(s.x),
                            i32::from(r.y) - i32::from(s.y),
                            i32::from(r.w),
                            i32::from(r.h),
                        );
                    }
                }
                match self.start_3d(at, clock) {
                    Some(due) => {
                        self.job = Job::ThreeD { back: false };
                        Step::Wait(due)
                    }
                    None => {
                        self.two_d.flip();
                        self.job = Job::None;
                        Step::Shown
                    }
                }
            }
            Job::ThreeD { back } => {
                let p = match &mut self.scene {
                    Some(s) => {
                        let (e, mem) = self.two_d.parts();
                        s.r.poll(&mut e.regs, mem, clock)
                    }
                    None => Poll::Idle,
                };
                self.after_3d();
                let shown = match p {
                    Poll::Wait(t) => return Step::Wait(t),
                    Poll::Recovered => Step::Recovered,
                    Poll::Done(_) | Poll::Idle => Step::Shown,
                };
                if back {
                    self.two_d.flip_back();
                } else {
                    self.two_d.flip();
                }
                self.job = Job::None;
                shown
            }
        }
    }

    /// The 2D engine's stream restarts after a reset the 3D renderer made
    /// (the reset is the whole MBX's).
    fn after_3d(&mut self) {
        let Some(s) = &mut self.scene else {
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
        let Some(s) = &mut self.scene else {
            return false;
        };
        let (e, _) = self.two_d.parts();
        s.r.idle(&mut e.regs, now_us)
    }
}

#[cfg(test)]
mod tests;
