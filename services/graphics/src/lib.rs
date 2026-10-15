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
//!   client.
//!
//! Who draws where when 2D and 3D share a picture (G1) waits for the
//! owner's vote; until then the caller gives each 3D render its target.
//!
//! A library now (G4), used in one process; the display driver host's once
//! IPC can move pages (ADR-0016).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_draw::{Drawn, Frame, Renderer as _};
use rp_hal_api::RegisterBus;
use rp_mbx_render::{LocalMem, Mbx2dRenderer};
use rp_mbx3d_render::{self as r3d, Clock, Depth, Draw, Paint, Point, Poll, Target, Texture};
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

/// The owner of the 2D engine and the 3D core.
pub struct Graphics<R, M> {
    two_d: Mbx2dRenderer<R, M>,
    scene: Option<(u8, r3d::Renderer)>,
    pool: Pool,
    clients: [ClientRec; CLIENTS],
    blocks: [Block; BLOCKS],
    resets: u32,
}

impl<R: RegisterBus, M: LocalMem> Graphics<R, M> {
    /// The service over the 2D renderer (which holds the register window
    /// and memory) and the pool.
    pub fn new(two_d: Mbx2dRenderer<R, M>, pool: Pool) -> Self {
        Self {
            two_d,
            scene: None,
            pool,
            clients: [ClientRec::default(); CLIENTS],
            blocks: [NO_BLOCK; BLOCKS],
            resets: 0,
        }
    }

    /// The 2D renderer (wallpaper, showing the picture: the service's own
    /// set-up).
    pub fn two_d(&mut self) -> &mut Mbx2dRenderer<R, M> {
        &mut self.two_d
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

    /// Ends a client: its blocks go back to the pool, its scene closes.
    pub fn close(&mut self, c: ClientId) -> Result<(), Error> {
        self.rec(c)?;
        if self.scene.as_ref().is_some_and(|s| s.0 == c.0) {
            self.close_scene(c)?;
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

    /// Uploads a texture for client `c` (see `rp_mbx3d_render::upload`),
    /// within its quota.
    pub fn upload_texture(
        &mut self,
        c: ClientId,
        code: u8,
        format: u32,
        texels: &[u16],
    ) -> Result<Handle, Error> {
        let rec = self.rec(c)?;
        let bytes = (8u32 << code.min(7)).pow(2) * 2;
        if rec.used.saturating_add(bytes) > rec.quota {
            return Err(Error::Quota);
        }
        let k = self
            .blocks
            .iter()
            .position(|b| b.tex.is_none())
            .ok_or(Error::NoMemory)?;
        let (_, mem) = self.two_d.parts();
        let t = r3d::upload(&mut self.pool, mem, code, format, texels).ok_or(Error::NoMemory)?;
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

    /// Gives a texture back. Refused while client `c`'s scene may still
    /// draw it: the caller empties the quads using it first.
    pub fn free(&mut self, c: ClientId, h: Handle) -> Result<(), Error> {
        self.rec(c)?;
        self.texture(c, h)?;
        if self.scene.as_ref().is_some_and(|s| s.1.busy()) {
            return Err(Error::Busy);
        }
        self.drop_block(usize::from(h.slot));
        Ok(())
    }

    /// Opens the 3D scene for client `c`, `w` x `h` pixels (whole tiles).
    pub fn open_scene(&mut self, c: ClientId, w: u16, h: u16) -> Result<(), Error> {
        self.rec(c)?;
        if self.scene.is_some() {
            return Err(Error::Busy);
        }
        let r = r3d::Renderer::new(&mut self.pool, w, h).ok_or(Error::NoMemory)?;
        self.scene = Some((c.0, r));
        Ok(())
    }

    /// Closes client `c`'s scene (when no render is in flight).
    pub fn close_scene(&mut self, c: ClientId) -> Result<(), Error> {
        match self.scene.take() {
            Some((o, r)) if o == c.0 && !r.busy() => {
                let _ = r.release(&mut self.pool);
                Ok(())
            }
            other => {
                let busy = other.as_ref().is_some_and(|s| s.0 == c.0);
                self.scene = other;
                Err(if busy {
                    Error::Busy
                } else {
                    Error::NotYourScene
                })
            }
        }
    }

    fn scene(&mut self, c: ClientId) -> Result<&mut r3d::Renderer, Error> {
        match &mut self.scene {
            Some((o, r)) if *o == c.0 => Ok(r),
            _ => Err(Error::NotYourScene),
        }
    }

    /// Client `c`'s scene background (see `Renderer::set_background`).
    pub fn set_background(&mut self, c: ClientId, argb: Option<u32>) -> Result<(), Error> {
        self.scene(c)?.set_background(argb).map_err(Error::Scene)
    }

    /// Puts `q` in slot `slot` of client `c`'s scene (or empties it); its
    /// texture must be one of `c`'s.
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
                    } => Paint::Texture {
                        tex: self.texture(c, tex)?.with_modes(u_mode, v_mode),
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
        let r = match &mut self.scene {
            Some((o, r)) if *o == c.0 => r,
            _ => return Err(Error::NotYourScene),
        };
        let (_, mem) = self.two_d.parts();
        r.set(mem, slot, draw.as_ref()).map_err(Error::Scene)
    }

    /// Queues a 2D frame (refused while a 3D render is in flight).
    pub fn draw2d(&mut self, frame: &Frame) -> Result<Drawn, Error> {
        if self.scene.as_ref().is_some_and(|s| s.1.busy()) {
            return Err(Error::Busy);
        }
        Ok(self.two_d.draw(frame))
    }

    /// Puts a finished 2D frame on screen; true when none is pending.
    pub fn poll2d(&mut self) -> bool {
        self.two_d.poll()
    }

    /// Starts rendering client `c`'s scene into `target` once the 2D
    /// engine is idle. Returns when to poll, or `None` if nothing changed.
    pub fn render3d<C: Clock>(
        &mut self,
        c: ClientId,
        clock: &mut C,
        target: Target,
    ) -> Result<Option<u32>, Error> {
        if !self.two_d.poll() {
            return Err(Error::Busy);
        }
        let r = match &mut self.scene {
            Some((o, r)) if *o == c.0 => r,
            _ => return Err(Error::NotYourScene),
        };
        let (e, mem) = self.two_d.parts();
        let res = r.render(&mut e.regs, mem, clock, target);
        self.after_3d();
        res.map_err(Error::Scene)
    }

    /// Checks on the 3D render in flight.
    pub fn poll3d<C: Clock>(&mut self, clock: &mut C) -> Poll {
        let Some((_, r)) = &mut self.scene else {
            return Poll::Idle;
        };
        let (e, mem) = self.two_d.parts();
        let p = r.poll(&mut e.regs, mem, clock);
        self.after_3d();
        p
    }

    /// The 2D engine's stream restarts after a reset the 3D renderer made
    /// (the reset is the whole MBX's).
    fn after_3d(&mut self) {
        let Some((_, r)) = &self.scene else {
            return;
        };
        if r.core_resets() != self.resets {
            self.resets = r.core_resets();
            self.two_d.parts().0.after_core_reset();
        }
    }

    /// When idle at `now_us`: the 3D clock off once unused for a while.
    pub fn idle(&mut self, now_us: u32) -> bool {
        let Some((_, r)) = &mut self.scene else {
            return false;
        };
        let (e, _) = self.two_d.parts();
        r.idle(&mut e.regs, now_us)
    }
}

#[cfg(test)]
mod tests;
