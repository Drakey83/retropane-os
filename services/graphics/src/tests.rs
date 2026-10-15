//! The service on both engines' models at once (one memory, one register
//! window): the 2D engine's (`rp-mbx-render`) and the 3D core's
//! (`rp-mbx3d-render`), each as measured on the X51v.

extern crate std;

use std::sync::atomic::{AtomicU32, Ordering};

use rp_draw::{Cmd, Frame, Rect};
use rp_drv_blit2d::{MBX, reg};
use rp_drv_mbx_display::clock::MBXCLK;
use rp_hal_api::RegisterBus;
use rp_mbx_render::model as m2;
use rp_mbx_render::{Layout, Mbx2dRenderer};
use rp_mbx3d_render::model::{self as m3, SimClock};
use rp_mbx3d_render::{Picture, format};
use rp_vram_map::POOL;

use super::*;

/// The register window: the 3D core's registers (and the clock) to its
/// model, the 2D engine's to its own; reset, enables and status to both.
struct Both {
    two: m2::Bus,
    three: m3::Gpu,
}

fn is_3d(off: usize) -> bool {
    off == MBXCLK || (MBX + 0x600..MBX + 0x900).contains(&off)
}

impl RegisterBus for Both {
    fn read32(&mut self, off: usize) -> u32 {
        if off == MBX + reg::STATUS {
            self.two.read32(off) | self.three.read32(off)
        } else if is_3d(off) || off == MBX + reg::ENABLE {
            self.three.read32(off)
        } else {
            self.two.read32(off)
        }
    }

    fn write32(&mut self, off: usize, v: u32) {
        if is_3d(off) {
            self.three.write32(off, v);
        } else if off == MBX + reg::RESET || off == MBX + reg::CLEAR || off == MBX + reg::ENABLE {
            self.two.write32(off, v);
            self.three.write32(off, v);
        } else {
            self.two.write32(off, v);
        }
    }
}

static NOW: AtomicU32 = AtomicU32::new(0);
fn now() -> u32 {
    NOW.fetch_add(1, Ordering::Relaxed)
}

const GREY: u32 = 0xFF20_2020;
const BLUE: u32 = 0xFF00_00FF;

struct Rig {
    g: Graphics<Both, m2::Mem>,
    mem: m2::Shared,
    clock: SimClock,
}

fn rig() -> Rig {
    let mem = m2::memory();
    let clock = SimClock::default();
    let bus = Both {
        two: m2::Bus::new(mem.clone()),
        three: m3::Gpu::new(mem.clone(), &clock),
    };
    let mut two = Mbx2dRenderer::new(bus, m2::Mem(mem.clone()), Layout::X51V, now, 1);
    let row = [GREY; 480];
    for y in 0..640 {
        two.wallpaper_row(y, &row);
    }
    two.show(|_| {});
    two.dither = false; // exact colours to compare
    Rig {
        g: Graphics::new(two, Pool::new(POOL)),
        mem,
        clock,
    }
}

impl Rig {
    fn px(&self, pic: Picture, x: u32, y: u32) -> u32 {
        let a = (pic.at + y * pic.stride + 4 * x) as usize;
        let m = self.mem.borrow();
        u32::from_le_bytes(m[a..a + 4].try_into().unwrap())
    }

    fn render(&mut self, c: ClientId, t: Target) {
        let due = self.g.render3d(c, &mut self.clock, t).unwrap().unwrap();
        self.clock.set(due);
        loop {
            match self.g.poll3d(&mut self.clock) {
                Poll::Done(_) => return,
                Poll::Wait(t) => self.clock.set(t),
                p => panic!("{p:?}"),
            }
        }
    }

    fn frame2d(&mut self, f: &Frame) {
        self.g.draw2d(f).unwrap();
        while !self.g.poll2d() {}
    }
}

fn texels() -> [u16; 64] {
    [0x001F; 64]
}

#[test]
fn clients_reach_only_their_own_memory_within_their_quota() {
    let mut r = rig();
    let a = r.g.client(256).unwrap();
    let b = r.g.client(1024).unwrap();
    let free = |r: &Rig| r.g.pool.free_bytes();
    let before = free(&r);
    let ta = r.g.upload_texture(a, 0, format::RGB565, &texels()).unwrap();
    assert_eq!(r.g.used(a), 128);
    assert_eq!(
        r.g.upload_texture(a, 1, format::RGB565, &[0; 256]),
        Err(Error::Quota)
    );
    r.g.upload_texture(a, 0, format::RGB565, &texels()).unwrap();
    assert_eq!(
        r.g.upload_texture(a, 0, format::RGB565, &texels()),
        Err(Error::Quota),
        "256 used"
    );
    // B can't name A's texture, in a quad or to free it.
    r.g.open_scene(b, 480, 640).unwrap();
    let q = Quad::rect(0, 0, 8, 16, Fill::texture(ta));
    assert_eq!(r.g.set(b, 0, Some(&q)), Err(Error::BadHandle));
    assert_eq!(r.g.free(b, ta), Err(Error::BadHandle));
    // Nor A B's scene.
    assert_eq!(r.g.set(a, 0, None), Err(Error::NotYourScene));
    assert_eq!(r.g.open_scene(a, 480, 640), Err(Error::Busy), "one scene");
    // A freed handle is dead, even if its slot is reused.
    r.g.free(a, ta).unwrap();
    assert_eq!(r.g.free(a, ta), Err(Error::BadHandle));
    let tb = r.g.upload_texture(b, 0, format::RGB565, &texels()).unwrap();
    assert_eq!(r.g.set(a, 0, None), Err(Error::NotYourScene));
    assert_eq!(r.g.free(a, tb), Err(Error::BadHandle), "slot reused by B");
    // Closing gives everything back.
    r.g.close(a).unwrap();
    r.g.close(b).unwrap();
    assert_eq!(free(&r), before);
    assert_eq!(r.g.used(a), 0);
    assert_eq!(
        r.g.upload_texture(a, 0, format::RGB565, &texels()),
        Err(Error::NoClient)
    );
}

#[test]
fn one_queue_and_both_engines_on_one_picture() {
    let mut r = rig();
    let c = r.g.client(64 * 1024).unwrap();
    r.g.open_scene(c, 64, 64).unwrap();
    r.g.set_background(c, Some(BLUE)).unwrap();
    let tex =
        r.g.upload_texture(c, 0, format::RGB565, &[0xF800; 64])
            .unwrap();
    r.g.set(c, 0, Some(&Quad::rect(16, 16, 32, 32, Fill::texture(tex))))
        .unwrap();
    // A 2D frame: a white bar across the top.
    let mut f = Frame::new();
    f.damage(Rect::new(0, 0, 480, 640));
    f.push(Cmd::Fill {
        r: Rect::new(0, 0, 480, 32),
        color: 0xFFFF_FFFF,
    });
    r.frame2d(&f);
    // The picture on screen, and the 3D scene drawn into a 64 x 64 area
    // of it (G1 pending: the caller picks the area).
    let scadr =
        r.g.two_d()
            .parts()
            .0
            .regs
            .two
            .reg(rp_drv_mbx_display::reg::GSCADR);
    let pic = Picture {
        at: (scadr & 0x7F_FFFF) * 16,
        ..Picture::screen(0)
    };
    let t = Target {
        picture: pic,
        x: 200,
        y: 304,
    };
    let due = r.g.render3d(c, &mut r.clock, t).unwrap().unwrap();
    // A 2D frame waits for the 3D render in flight.
    assert_eq!(r.g.draw2d(&f).err(), Some(Error::Busy));
    r.clock.set(due);
    while !matches!(r.g.poll3d(&mut r.clock), Poll::Done(_)) {}
    assert_eq!(r.px(pic, 5, 5) & 0xFF_FFFF, 0xFF_FFFF, "the 2D bar");
    assert_eq!(r.px(pic, 210, 310), BLUE, "the scene's background");
    assert_eq!(r.px(pic, 232, 336), 0xFFFF_0000, "the textured quad");
    assert_eq!(
        r.px(pic, 199, 310) & 0xFF_FFFF,
        GREY & 0xFF_FFFF,
        "beside the area: 2D's"
    );
    assert_eq!(r.px(pic, 264, 310) & 0xFF_FFFF, GREY & 0xFF_FFFF);
}

#[test]
fn a_3d_hang_resets_the_mbx_and_the_2d_engine_carries_on() {
    let mut r = rig();
    let c = r.g.client(0).unwrap();
    r.g.open_scene(c, 32, 32).unwrap();
    let t = Target {
        picture: Picture::screen(1),
        x: 0,
        y: 0,
    };
    r.render(c, t);
    r.g.set(c, 0, Some(&Quad::rect(0, 0, 8, 16, Fill::Flat(BLUE))))
        .unwrap();
    r.g.two_d().parts().0.regs.three.hang_next = true;
    let due = r.g.render3d(c, &mut r.clock, t).unwrap().unwrap();
    r.clock.set(due);
    let mut recovered = false;
    for _ in 0..5000 {
        match r.g.poll3d(&mut r.clock) {
            Poll::Wait(at) => r.clock.set(at),
            Poll::Recovered => {
                recovered = true;
                break;
            }
            p => panic!("{p:?}"),
        }
    }
    assert!(recovered);
    // The 2D engine works after the shared reset.
    let mut f = Frame::new();
    f.damage(Rect::new(0, 0, 480, 640));
    f.push(Cmd::Fill {
        r: Rect::new(0, 600, 480, 40),
        color: 0xFF00_FF00,
    });
    r.frame2d(&f);
    // And the 3D core again, everything redrawn.
    r.render(c, t);
    assert_eq!(r.px(Picture::screen(1), 2, 2), BLUE);
}

#[test]
fn closing_a_client_closes_its_scene_and_the_clock_goes_off_when_idle() {
    let mut r = rig();
    let c = r.g.client(0).unwrap();
    assert!(!r.g.idle(0), "no scene");
    r.g.open_scene(c, 32, 32).unwrap();
    let t = Target {
        picture: Picture::screen(0),
        x: 0,
        y: 0,
    };
    r.render(c, t);
    let later = r.clock.0.get() + 200_000;
    assert!(r.g.idle(later));
    assert_eq!(r.g.two_d().parts().0.regs.read32(MBXCLK) & 3, 1, "2D only");
    r.g.close(c).unwrap();
    let d = r.g.client(0).unwrap();
    r.g.open_scene(d, 32, 32).unwrap();
}
