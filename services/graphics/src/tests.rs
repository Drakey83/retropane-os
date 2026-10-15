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
const WHITE: u32 = 0xFFFF_FFFF;

struct Rig {
    g: Graphics<'static, Both, m2::Mem>,
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
        // Leaked: a test's renderer lives as long as the test process.
        g: Graphics::new(
            std::boxed::Box::leak(std::boxed::Box::new(two)),
            Pool::new(POOL),
        ),
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

    /// The picture on screen.
    fn shown(&mut self) -> Picture {
        let scadr = self
            .g
            .two_d()
            .parts()
            .0
            .regs
            .two
            .reg(rp_drv_mbx_display::reg::GSCADR);
        Picture {
            at: (scadr & 0x7F_FFFF) * 16,
            ..Picture::screen(0)
        }
    }

    /// A pixel of the picture on screen (colour only).
    fn on_screen(&mut self, x: u32, y: u32) -> u32 {
        let pic = self.shown();
        self.px(pic, x, y) & 0xFF_FFFF
    }

    /// Runs a frame to its end.
    fn frame(&mut self, f: &Frame) -> Step {
        let mut step = self.g.frame(f, &mut self.clock).unwrap();
        for _ in 0..10_000 {
            match step {
                Step::Wait(t) => {
                    self.clock.set(t);
                    step = self.g.poll(&mut self.clock);
                }
                s => return s,
            }
        }
        panic!("never finished");
    }
}

fn texels() -> [u16; 64] {
    [0x001F; 64]
}

/// A 2D frame: a fill over `r`, damaged.
fn fill(r: Rect, c: u32) -> Frame {
    let mut f = Frame::new();
    f.damage(r);
    f.push(Cmd::Fill { r, color: c });
    f
}

/// The shell's frames carry the whole scene, `damage` only what changed:
/// a white bar, then a green band across the 3D area.
fn scene_2d(green: bool, damage: Option<Rect>) -> Frame {
    let mut f = Frame::new();
    if let Some(r) = damage {
        f.damage(r);
    }
    f.push(Cmd::Fill {
        r: Rect::new(0, 0, 480, 32),
        color: WHITE,
    });
    if green {
        f.push(Cmd::Fill {
            r: Rect::new(0, 300, 480, 80),
            color: 0xFF00_FF00,
        });
    }
    f
}

const AREA: (u16, u16, u16, u16) = (200, 304, 64, 64);

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
    let (x, y, w, h) = AREA;
    r.g.open_scene(b, x, y, w, h).unwrap();
    let q = Quad::rect(0, 0, 8, 16, Fill::texture(ta));
    assert_eq!(r.g.set(b, 0, Some(&q)), Err(Error::BadHandle));
    assert_eq!(r.g.free(b, ta), Err(Error::BadHandle));
    // Nor A B's scene; one scene at a time.
    assert_eq!(r.g.set(a, 0, None), Err(Error::NotYourScene));
    assert_eq!(r.g.open_scene(a, x, y, w, h), Err(Error::Busy));
    // A freed handle is dead, even if its slot is reused.
    r.g.free(a, ta).unwrap();
    assert_eq!(r.g.free(a, ta), Err(Error::BadHandle));
    let tb = r.g.upload_texture(b, 0, format::RGB565, &texels()).unwrap();
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
fn the_scene_owns_its_area_and_nothing_half_drawn_reaches_the_screen() {
    let mut r = rig();
    let c = r.g.client(64 * 1024).unwrap();
    let (x, y, w, h) = AREA;
    r.g.open_scene(c, x, y, w, h).unwrap();
    r.g.set_background(c, Some(BLUE)).unwrap();
    let tex =
        r.g.upload_texture(c, 0, format::RGB565, &[0xF800; 64])
            .unwrap();
    r.g.set(c, 0, Some(&Quad::rect(16, 16, 32, 32, Fill::texture(tex))))
        .unwrap();
    // The first frame: 2D's white bar and its wallpaper, the scene's area.
    assert_eq!(
        r.frame(&scene_2d(false, Some(Rect::new(0, 0, 480, 32)))),
        Step::Shown
    );
    let check = |r: &mut Rig, quad_at: u32| {
        assert_eq!(r.on_screen(5, 5), WHITE & 0xFF_FFFF, "2D's bar");
        assert_eq!(
            r.on_screen(210, 310),
            BLUE & 0xFF_FFFF,
            "the scene's background"
        );
        assert_eq!(r.on_screen(quad_at, 336), 0xFF_0000, "the textured quad");
        assert_eq!(
            r.on_screen(199, 310),
            GREY & 0xFF_FFFF,
            "left of the area: 2D's"
        );
        assert_eq!(r.on_screen(264, 310), GREY & 0xFF_FFFF, "right of it");
    };
    check(&mut r, 232);
    // 2D draws a fill right across the area: the area stays the scene's,
    // in this frame and the next (both pictures).
    for _ in 0..2 {
        assert_eq!(
            r.frame(&scene_2d(true, Some(Rect::new(0, 300, 480, 80)))),
            Step::Shown
        );
        assert_eq!(r.on_screen(210, 310), BLUE & 0xFF_FFFF);
        assert_eq!(r.on_screen(5, 310), 0x00_FF00, "2D's fill beside it");
    }
    // A change in the scene only: shown with no 2D frame, 2D intact.
    r.g.set(c, 0, Some(&Quad::rect(24, 16, 32, 32, Fill::texture(tex))))
        .unwrap();
    let pic0 = r.shown();
    assert_eq!(r.frame(&scene_2d(true, None)), Step::Shown);
    assert_ne!(r.shown(), pic0, "flipped to the other picture");
    assert_eq!(r.on_screen(240, 336), 0xFF_0000);
    assert_eq!(r.on_screen(218, 336), BLUE & 0xFF_FFFF, "where it was");
    assert_eq!(
        r.on_screen(5, 310),
        0x00_FF00,
        "2D's frame in that picture too"
    );
    assert_eq!(r.on_screen(5, 5), WHITE & 0xFF_FFFF);
    // Nothing changed: nothing to do.
    assert_eq!(r.frame(&scene_2d(true, None)), Step::Idle);
    assert_eq!(r.frame(&scene_2d(true, None)), Step::Idle);
}

#[test]
fn one_job_at_a_time() {
    let mut r = rig();
    let c = r.g.client(0).unwrap();
    let (x, y, w, h) = AREA;
    r.g.open_scene(c, x, y, w, h).unwrap();
    let step =
        r.g.frame(&fill(Rect::new(0, 0, 480, 32), WHITE), &mut r.clock);
    assert!(matches!(step, Ok(Step::Wait(_))), "{step:?}");
    assert_eq!(r.g.frame(&Frame::new(), &mut r.clock), Err(Error::Busy));
    assert_eq!(r.g.set(c, 0, None), Err(Error::Busy));
    assert_eq!(r.g.close_scene(c), Err(Error::Busy));
    assert!(!r.g.idle(u32::MAX / 2));
}

#[test]
fn a_3d_hang_resets_the_mbx_and_the_2d_engine_carries_on() {
    let mut r = rig();
    let c = r.g.client(0).unwrap();
    let (x, y, w, h) = AREA;
    r.g.open_scene(c, x, y, w, h).unwrap();
    assert_eq!(r.frame(&Frame::new()), Step::Shown);
    r.g.set(c, 0, Some(&Quad::rect(0, 0, 8, 16, Fill::Flat(BLUE))))
        .unwrap();
    r.g.two_d().parts().0.regs.three.hang_next = true;
    assert_eq!(r.frame(&Frame::new()), Step::Recovered);
    // The 2D engine works after the shared reset, and the scene comes
    // back whole.
    assert_eq!(
        r.frame(&fill(Rect::new(0, 600, 480, 40), 0xFF00_FF00)),
        Step::Shown
    );
    assert_eq!(r.on_screen(5, 610), 0x00_FF00);
    assert_eq!(r.on_screen(202, 306), BLUE & 0xFF_FFFF);
}

#[test]
fn closing_the_scene_gives_its_area_back_and_the_clock_goes_off_when_idle() {
    let mut r = rig();
    let c = r.g.client(0).unwrap();
    assert!(!r.g.idle(0), "no scene");
    let (x, y, w, h) = AREA;
    r.g.open_scene(c, x, y, w, h).unwrap();
    r.g.set_background(c, Some(BLUE)).unwrap();
    assert_eq!(r.frame(&Frame::new()), Step::Shown);
    assert_eq!(r.on_screen(210, 310), BLUE & 0xFF_FFFF);
    let later = r.clock.0.get() + 200_000;
    assert!(r.g.idle(later));
    assert_eq!(r.g.two_d().parts().0.regs.read32(MBXCLK) & 3, 1, "2D only");
    r.g.close(c).unwrap();
    // The next frames redraw the area as 2D's.
    assert_eq!(r.frame(&Frame::new()), Step::Shown);
    assert_eq!(r.on_screen(210, 310), GREY & 0xFF_FFFF);
    let d = r.g.client(0).unwrap();
    r.g.open_scene(d, x, y, w, h).unwrap();
}

#[test]
fn the_service_stays_small() {
    // It lives on an image's stack next to the 2D renderer and a frame
    // (rp-today overflowed its 24 KiB when the service held the renderer:
    // 11 KiB here then). Host sizes (64-bit) bound the device's.
    let size = core::mem::size_of::<Graphics<'static, Both, m2::Mem>>();
    assert!(size <= 6 * 1024, "{size} bytes");
}
