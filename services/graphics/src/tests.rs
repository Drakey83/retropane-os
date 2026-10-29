//! The service on both engines' models at once (one memory, one register
//! window): the 2D engine's (`rp-mbx-render`) and the 3D core's
//! (`rp-mbx3d-render`), each as measured on the X51v.

extern crate std;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use rp_draw::{Cmd, Frame, Rect};
use rp_drv_blit2d::{MBX, reg};
use rp_drv_mbx_display::clock::MBXCLK;
use rp_hal_api::RegisterBus;
use rp_mbx_render::model as m2;
use rp_mbx_render::{Layout, Mbx2dRenderer};
use rp_mbx3d_render::model::{self as m3, SimClock};
use rp_mbx3d_render::{Picture, format};
use rp_vram_map::{POOL, SCREENS};

use super::*;

/// The register window: the 3D core's registers (and the clock) to its
/// model, the 2D engine's to its own; reset, enables and status to both.
/// The display's scan modelled on the test's clock (R20): `DVLNUM` counts
/// [`LINE_NS`] lines of a 645-line refresh, `DISPPLL` reads [`Both::pll`],
/// `VEVENT0` is set once a refresh has ended since it was cleared.
struct Both {
    two: m2::Bus,
    three: m3::Gpu,
    now: Rc<Cell<u32>>,
    pll: u32,
    acked: u32,
}

/// The modelled scan's line: RetroPane's 60 Hz set-up as measured (R111).
const LINE_NS: u64 = 25_687;

fn both(mem: &m2::Shared, clock: &SimClock) -> Both {
    Both {
        two: m2::Bus::new(mem.clone()),
        three: m3::Gpu::new(mem.clone(), clock),
        now: clock.0.clone(),
        pll: rp_drv_mbx_display::pll::SIXTY,
        acked: 0,
    }
}

impl Both {
    /// The scan's line count since time 0.
    fn lines(&self) -> u64 {
        u64::from(self.now.get()) * 1000 / LINE_NS
    }
}

fn is_3d(off: usize) -> bool {
    off == MBXCLK || (MBX + 0x600..MBX + 0x900).contains(&off)
}

impl RegisterBus for Both {
    fn read32(&mut self, off: usize) -> u32 {
        use rp_drv_mbx_display::{event::VEVENT0, reg as dreg};
        if off == dreg::DVLNUM {
            return (self.lines() % 645) as u32;
        }
        if off == dreg::DISPPLL {
            return self.pll;
        }
        if off == dreg::DINTRS {
            let ended = self.lines() / 645 > u64::from(self.acked) * 1000 / LINE_NS / 645;
            return self.two.read32(off) & !VEVENT0 | if ended { VEVENT0 } else { 0 };
        }
        if off == MBX + reg::STATUS {
            self.two.read32(off) | self.three.read32(off)
        } else if is_3d(off) || off == MBX + reg::ENABLE {
            self.three.read32(off)
        } else {
            self.two.read32(off)
        }
    }

    fn write32(&mut self, off: usize, v: u32) {
        if off == rp_drv_mbx_display::reg::DINTRS && v & rp_drv_mbx_display::event::VEVENT0 == 0 {
            self.acked = self.now.get();
        }
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

/// Exact in RGB565 (the screens' format since ADR-0023's 16-bit vote), as
/// every colour these tests check is.
const GREY: u32 = 0xFF21_2021;
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
    let bus = both(&mem, &clock);
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
            Some(std::boxed::Box::leak(std::boxed::Box::new(ThreeD::new()))),
        ),
        mem,
        clock,
    }
}

impl Rig {
    /// A pixel of `pic` as 8-bit xRGB (an RGB565 one widened as the
    /// engines read it).
    fn px(&self, pic: Picture, x: u32, y: u32) -> u32 {
        let a = (pic.at + y * pic.stride + pic.format.bytes() * x) as usize;
        let m = self.mem.borrow();
        if pic.format.bytes() == 2 {
            m2::widen(u16::from_le_bytes([m[a], m[a + 1]]))
        } else {
            u32::from_le_bytes(m[a..a + 4].try_into().unwrap())
        }
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
    // The scene renders into the 16-bit screen with the core's dither
    // (`0x650` 9), at the screen's stride (`0x660`: bytes / 4).
    let three = &r.g.two_d().parts().0.regs.three;
    assert_eq!((three.reg(0x650), three.reg(0x660)), (9, 480 * 2 / 4));
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
    assert_eq!(r.frame(&scene_2d(true, None)), Step::Shown);
    assert_eq!(
        r.shown().at,
        SCREENS[0].at,
        "rendered into screen 0, on screen"
    );
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

/// C71 ruling R-B: the 3D core renders only into screen 0 (inside the safe
/// region) while screen 0 is on screen; screen 1 stays 2D's back picture,
/// and the scene's area is carried into it, so a 2D frame flipped there
/// shows the scene current.
#[test]
fn the_3d_core_draws_only_into_screen_0_on_screen() {
    let mut r = rig();
    let c = r.g.client(64 * 1024).unwrap();
    let (x, y, w, h) = AREA;
    r.g.open_scene(c, x, y, w, h).unwrap();
    r.g.set_background(c, Some(BLUE)).unwrap();
    let tex =
        r.g.upload_texture(c, 0, format::RGB565, &[0xF800; 64])
            .unwrap();
    let both = |r: &mut Rig, px: u32, py: u32| {
        let s0 = Picture {
            at: SCREENS[0].at,
            ..Picture::screen(0)
        };
        let s1 = Picture {
            at: SCREENS[1].at,
            ..Picture::screen(0)
        };
        (r.px(s0, px, py) & 0xFF_FFFF, r.px(s1, px, py) & 0xFF_FFFF)
    };
    for k in 0..6u16 {
        // Odd frames move the quad (3D work), even ones change 2D only
        // outside the area; every third has both.
        let moved = k % 2 == 1 || k % 3 == 0;
        if moved {
            r.g.set(
                c,
                0,
                Some(&Quad::rect(
                    8 + 4 * i32::from(k),
                    16,
                    32,
                    32,
                    Fill::texture(tex),
                )),
            )
            .unwrap();
        }
        let bar = Rect::new(0, 0, 480, 32);
        let colour = if k % 2 == 0 { WHITE } else { 0xFF00_FF00 };
        let mut f = Frame::new();
        f.damage(bar);
        f.push(Cmd::Fill {
            r: bar,
            color: colour,
        });
        assert_eq!(r.frame(&f), Step::Shown, "frame {k}");
        if moved {
            assert_eq!(
                r.shown().at,
                SCREENS[0].at,
                "frame {k}: 3D shown from screen 0"
            );
        }
        // Both screens hold the whole frame: 2D's bar, the scene's quad.
        assert_eq!(r.on_screen(5, 5), colour & 0xFF_FFFF, "frame {k}");
        let (a, b) = both(&mut r, 210, 310);
        assert_eq!(
            (a, b),
            (BLUE & 0xFF_FFFF, BLUE & 0xFF_FFFF),
            "frame {k}: the background"
        );
    }
    // The last quad, at 8 + 4 * 5 = 28 in the area, in both screens.
    let at = u32::from(x) + 28 + 16;
    assert_eq!(
        both(&mut r, at, 336),
        (0xFF_0000, 0xFF_0000),
        "the quad carried"
    );
    // A 2D frame with no 3D work may flip to screen 1: the scene is current
    // there.
    let bar = Rect::new(0, 0, 480, 32);
    let mut f = Frame::new();
    f.damage(bar);
    f.push(Cmd::Fill {
        r: bar,
        color: WHITE,
    });
    assert_eq!(r.frame(&f), Step::Shown);
    assert_eq!(r.on_screen(at, 336), 0xFF_0000);
    assert_eq!(r.on_screen(210, 310), BLUE & 0xFF_FFFF);
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

/// The model engine's FIFO reported full for good: the next submit stalls.
fn stall(two: &mut Mbx2dRenderer<Both, m2::Mem>) {
    two.parts().0.regs.two.full_reads = u32::MAX;
}

/// A frame that moves the scene's quad to `qx` and fills the top bar.
fn moved(r: &mut Rig, c: ClientId, tex: Handle, qx: i32, colour: u32) -> Frame {
    r.g.set(c, 0, Some(&Quad::rect(qx, 16, 32, 32, Fill::texture(tex))))
        .unwrap();
    let bar = Rect::new(0, 0, 480, 32);
    let mut f = Frame::new();
    f.damage(bar);
    f.push(Cmd::Fill {
        r: bar,
        color: colour,
    });
    f
}

#[test]
fn a_stalled_copy_between_the_screens_is_reported_and_the_next_frame_whole() {
    // The frame's first copy (2D's changes into screen 0, before the
    // render) and its second (the area into screen 1, after it).
    for nth in [1u8, 2] {
        let mut r = rig();
        let c = r.g.client(64 * 1024).unwrap();
        let (x, y, w, h) = AREA;
        r.g.open_scene(c, x, y, w, h).unwrap();
        r.g.set_background(c, Some(BLUE)).unwrap();
        let tex =
            r.g.upload_texture(c, 0, format::RGB565, &[0xF800; 64])
                .unwrap();
        // Two frames so screen 0 is on screen and screen 1 drawn into.
        let f = moved(&mut r, c, tex, 8, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
        let f = moved(&mut r, c, tex, 12, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
        assert_eq!(r.g.copy_stalls(), 0);
        // The stalled frame is reported, never Shown.
        let f = moved(&mut r, c, tex, 20, 0xFF00_FF00);
        r.g.before_copy = Some((nth, stall));
        let step = r.frame(&f);
        r.g.two_d().parts().0.regs.two.full_reads = 0;
        assert_eq!(step, Step::Recovered, "copy {nth}");
        assert_eq!(r.g.copy_stalls(), 1, "copy {nth}: counted");
        // The next frame (the client's same list) shows it all, on both
        // screens.
        assert_eq!(r.frame(&f), Step::Shown, "copy {nth}");
        let qx = u32::from(x) + 20 + 16;
        assert_eq!(r.on_screen(qx, 336), 0xFF_0000, "copy {nth}: the quad");
        assert_eq!(r.on_screen(5, 5), 0x00_FF00, "copy {nth}: 2D's bar");
        for s in SCREENS {
            let pic = Picture {
                at: s.at,
                ..Picture::screen(0)
            };
            assert_eq!(r.px(pic, qx, 336) & 0xFF_FFFF, 0xFF_0000, "copy {nth}");
            assert_eq!(r.px(pic, 5, 5) & 0xFF_FFFF, 0x00_FF00, "copy {nth}");
        }
        assert_eq!(r.g.copy_stalls(), 1, "copy {nth}");
    }
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
    // It lives next to the 2D renderer and a frame (rp-today overflowed its
    // 24 KiB stack when the service held the renderer: 11 KiB here then;
    // 5.4 KiB with the 3D state inside, 2026-10-17). The 3D state is lent
    // only by programs that use 3D. Host sizes (64-bit) bound the device's.
    let size = core::mem::size_of::<Graphics<'static, Both, m2::Mem>>();
    assert!(size <= 1024, "{size} bytes");
}

#[test]
fn without_3d_state_frames_are_2d_and_3d_is_refused() {
    let mem = m2::memory();
    let clock = SimClock::default();
    let bus = both(&mem, &clock);
    let mut two = Mbx2dRenderer::new(bus, m2::Mem(mem.clone()), Layout::X51V, now, 1);
    let row = [GREY; 480];
    for y in 0..640 {
        two.wallpaper_row(y, &row);
    }
    two.show(|_| {});
    two.dither = false;
    let mut r = Rig {
        g: Graphics::new(
            std::boxed::Box::leak(std::boxed::Box::new(two)),
            Pool::new(POOL),
            None,
        ),
        mem,
        clock,
    };
    let c = r.g.client(4096).unwrap();
    let (x, y, w, h) = AREA;
    assert_eq!(r.g.open_scene(c, x, y, w, h), Err(Error::No3d));
    assert_eq!(
        r.g.upload_texture(c, 0, format::RGB565, &texels()),
        Err(Error::No3d)
    );
    assert_eq!(r.g.used(c), 0);
    assert_eq!(r.frame(&fill(Rect::new(0, 0, 480, 32), BLUE)), Step::Shown);
    assert_eq!(r.on_screen(10, 10), BLUE & 0xFF_FFFF);
    assert!(!r.g.idle(0));
    r.g.close(c).unwrap();
}

#[test]
fn compressed_textures_count_at_their_size() {
    // PVRTC 4 bpp: a 16 x 16 texture is 128 bytes against the quota (16-bit
    // texels: 512); 32-bit texels count 4 a texel.
    let mut r = rig();
    let c = r.g.client(128 + 256).unwrap();
    let blocks = [0u32, 0xFC00_FC00].repeat(16);
    r.g.upload_pvrtc(c, 1, format::PVRTC4, &blocks).unwrap();
    assert_eq!(r.g.used(c), 128);
    r.g.upload_texture32(c, 0, &[0xFFFF_0000; 64]).unwrap();
    assert_eq!(r.g.used(c), 384);
    assert_eq!(
        r.g.upload_texture(c, 0, format::RGB565, &texels()),
        Err(Error::Quota)
    );
}

#[test]
fn mipmapped_textures_count_their_levels() {
    let mut r = rig();
    let c = r.g.client(172).unwrap();
    r.g.upload_mipmapped(c, 0, format::RGB565, &texels())
        .unwrap();
    assert_eq!(r.g.used(c), 128 + 32 + 8 + 4);
    assert_eq!(
        r.g.upload_mipmapped(c, 0, format::RGB565, &texels()),
        Err(Error::Quota)
    );
}

/// The scene's renderer (R20's tests set its tile order).
fn scene_r(r: &mut Rig) -> &mut r3d::Renderer {
    &mut r
        .g
        .three_d
        .as_deref_mut()
        .unwrap()
        .scene
        .as_mut()
        .unwrap()
        .r
}

/// A scene in [`AREA`] on a blue background with a red quad: its texture.
fn scene(r: &mut Rig) -> (ClientId, Handle) {
    let c = r.g.client(64 * 1024).unwrap();
    let (x, y, w, h) = AREA;
    r.g.open_scene(c, x, y, w, h).unwrap();
    r.g.set_background(c, Some(BLUE)).unwrap();
    let tex =
        r.g.upload_texture(c, 0, format::RGB565, &[0xF800; 64])
            .unwrap();
    (c, tex)
}

/// Both screens hold the quad at `qx` in the area and 2D's bar `bar`.
fn both_hold(r: &Rig, qx: i32, bar: u32, what: &str) {
    let x = u32::from(AREA.0) + qx as u32 + 16;
    for (k, s) in SCREENS.iter().enumerate() {
        let pic = Picture {
            at: s.at,
            ..Picture::screen(0)
        };
        let px = |x, y| r.px(pic, x, y) & 0xFF_FFFF;
        assert_eq!(px(x, 336), 0xFF_0000, "{what}: the quad in screen {k}");
        assert_eq!(px(210, 310), BLUE & 0xFF_FFFF, "{what}: screen {k}");
        assert_eq!(px(5, 5), bar & 0xFF_FFFF, "{what}: 2D in screen {k}");
    }
}

/// The 2D scene `moved` draws, unchanged (no damage): a frame with the
/// 3D scene's change only.
fn still(colour: u32) -> Frame {
    let mut f = Frame::new();
    f.push(Cmd::Fill {
        r: Rect::new(0, 0, 480, 32),
        color: colour,
    });
    f
}

/// The scene's band of screen rows.
fn band() -> (u32, u32) {
    (u32::from(AREA.1), u32::from(AREA.3))
}

#[test]
fn frames_the_scan_can_lead_start_behind_it_and_never_tear() {
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    let mut seen_behind = 0;
    for k in 0..12 {
        let f = moved(&mut r, c, tex, 4 + 2 * k, WHITE);
        let before = r.g.starts();
        let plan = r.g.sched.window(&scan::X51V_60HZ, true, band());
        assert_eq!(r.frame(&f), Step::Shown, "frame {k}");
        let after = r.g.starts();
        let line = after.last_line.unwrap();
        if let Some((from, to)) = plan {
            // Started inside its window, behind the scan, on screen 0.
            assert_eq!(after.behind, before.behind + 1, "frame {k}");
            assert!(
                (from..=to).contains(&line),
                "frame {k}: {line} in {from}-{to}"
            );
            let (fast, slow) = r.g.sched.band().unwrap();
            for t in [fast, slow] {
                assert!(
                    scan::tear_free_rows(&scan::X51V_60HZ, band(), line, t),
                    "frame {k}: a {t} us render from line {line}"
                );
            }
            seen_behind += 1;
        } else {
            // No history yet: the fallback, once the scan has left the area.
            assert_eq!(after.after_area, before.after_area + 1, "frame {k}");
            let end = band().0 + band().1;
            assert!((end..=end + AFTER_AREA_LINES).contains(&line), "{line}");
        }
        assert_eq!(r.shown().at, SCREENS[0].at, "frame {k}: screen 0");
        both_hold(&r, 4 + 2 * k, WHITE, "frame");
    }
    assert!(seen_behind >= 10, "{seen_behind}");
    assert_eq!(r.g.starts().screen1, 0);
}

#[test]
fn before_screen_1_is_proven_unplaceable_frames_start_after_the_area() {
    // A checkerboard order (the divide-1 cure) never starts behind the
    // scan; with option 1 off, every such frame starts on screen 0 once
    // the scan has left the area.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    for k in 0..4 {
        let f = moved(&mut r, c, tex, 8 + 4 * k, WHITE);
        assert_eq!(r.frame(&f), Step::Shown, "frame {k}");
        let line = r.g.starts().last_line.unwrap();
        let end = band().0 + band().1;
        assert!(
            (end..=end + AFTER_AREA_LINES).contains(&line),
            "frame {k}: {line}"
        );
        assert_eq!(r.shown().at, SCREENS[0].at);
        both_hold(&r, 8 + 4 * k, WHITE, "after the area");
    }
    let st = r.g.starts();
    assert_eq!((st.behind, st.after_area, st.screen1), (0, 4, 0));
}

#[test]
fn once_screen_1_is_proven_unplaceable_frames_render_there_and_flip() {
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    r.g.allow_screen1_3d(true);
    // Frames with a 2D change (2D's frame drawn into either screen) and
    // one with the scene's change only.
    for (k, two_d) in [(0, true), (1, true), (2, false), (3, true), (4, false)] {
        let colour = if k % 2 == 0 { WHITE } else { 0xFF00_FF00 };
        let mut f = moved(&mut r, c, tex, 8 + 4 * k, colour);
        if !two_d {
            f = still(0xFF00_FF00);
        }
        // The render starts into screen 1 with screen 0 on screen.
        let mut step = r.g.frame(&f, &mut r.clock).unwrap();
        let mut aside_seen = false;
        let mut flipped_in = None;
        let refresh = |r: &Rig| r.clock.0.get() as u64 * 1000 / LINE_NS / 645;
        for _ in 0..10_000 {
            if let Job::Aside { .. } = r.g.job {
                aside_seen = true;
                assert_eq!(
                    r.shown().at,
                    SCREENS[0].at,
                    "frame {k}: rendering, screen 0 shown"
                );
            }
            if let Job::Flipped { .. } = r.g.job {
                flipped_in.get_or_insert(refresh(&r));
            }
            match step {
                Step::Wait(t) => {
                    r.clock.set(t);
                    step = r.g.poll(&mut r.clock);
                }
                _ => break,
            }
        }
        assert_eq!(step, Step::Shown, "frame {k}");
        assert!(aside_seen, "frame {k}: rendered into screen 1");
        // Screen 0 was copied into only once the flip's refresh had ended
        // (scan-out off it).
        let flipped_in = flipped_in.expect("flipped");
        assert!(
            refresh(&r) > flipped_in,
            "frame {k}: copied during the refresh"
        );
        assert_eq!(
            r.shown().at,
            SCREENS[1].at,
            "frame {k}: flipped to screen 1"
        );
        let bar = if two_d { colour } else { 0xFF00_FF00 };
        both_hold(&r, 8 + 4 * k, bar, &format!("option 1 frame {k}"));
    }
    let st = r.g.starts();
    assert_eq!((st.behind, st.after_area, st.screen1), (0, 0, 5));
    assert_eq!(r.g.copy_stalls(), 0);
}

#[test]
fn a_spike_takes_the_fallback_and_the_scan_is_led_again_after() {
    // The owner's rule: a frame suddenly much longer than the last must
    // never overtake the scan. After steady 6 ms frames the band allows a
    // 12 ms spike; a 30 ms one leaves no window: the next frame takes the
    // fallback, and the one after that starts behind the scan again.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    r.g.sched.reset();
    for _ in 0..8 {
        r.g.sched.record(6_000);
    }
    assert!(matches!(
        r.g.plan_3d(true),
        Some(Start::Screen0 { behind: true, .. })
    ));
    let (fast, slow) = r.g.sched.band().unwrap();
    assert!(slow >= 12_000, "{slow}");
    let (from, to) = r.g.sched.window(&scan::X51V_60HZ, true, band()).unwrap();
    for line in [from, to] {
        assert!(scan::tear_free_rows(&scan::X51V_60HZ, band(), line, 12_000));
        assert!(scan::tear_free_rows(&scan::X51V_60HZ, band(), line, fast));
    }
    r.g.sched.record(30_000);
    assert!(matches!(
        r.g.plan_3d(true),
        Some(Start::Screen0 { behind: false, .. })
    ));
    r.g.allow_screen1_3d(true);
    assert_eq!(r.g.plan_3d(true), Some(Start::Screen1));
    // A real frame after the spike: option 1, then its own (fast) time
    // brings the scan back.
    let f = moved(&mut r, c, tex, 12, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    assert_eq!(r.g.starts().screen1, 1);
    for _ in 0..8 {
        r.g.sched.record(6_000);
    }
    let f = moved(&mut r, c, tex, 16, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    assert_eq!(r.g.starts().behind, 1);
    assert_eq!(r.shown().at, SCREENS[0].at);
    both_hold(&r, 16, WHITE, "after the spike");
}

#[test]
fn an_unmeasured_pixel_clock_never_starts_behind_the_scan() {
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    r.g.two_d().parts().0.regs.pll = 0x1234;
    for _ in 0..8 {
        r.g.sched.record(6_000);
    }
    for k in 0..3 {
        let f = moved(&mut r, c, tex, 8 + 4 * k, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
    }
    let st = r.g.starts();
    assert_eq!((st.behind, st.after_area), (0, 3));
    // Windows Mobile's 56 Hz has its own measured scan.
    r.g.two_d().parts().0.regs.pll = rp_drv_mbx_display::pll::WM;
    let f = moved(&mut r, c, tex, 24, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    assert_eq!(r.g.starts().behind, 1);
}

#[test]
fn a_hang_in_screen_1_keeps_screen_0_shown_and_the_next_frame_whole() {
    for two_d in [true, false] {
        let mut r = rig();
        let (c, tex) = scene(&mut r);
        scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
        r.g.allow_screen1_3d(true);
        let f = moved(&mut r, c, tex, 8, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
        // Back to screen 0 for the next 3D frame.
        let mut f = moved(&mut r, c, tex, 12, 0xFF00_FF00);
        if !two_d {
            f = still(WHITE);
        }
        r.g.two_d().parts().0.regs.three.hang_next = true;
        assert_eq!(r.frame(&f), Step::Recovered, "2D {two_d}");
        assert_eq!(r.shown().at, SCREENS[0].at, "2D {two_d}: screen 0 kept");
        if two_d {
            assert_eq!(r.on_screen(5, 5), 0x00_FF00, "2D's frame shown");
        }
        // The schedule forgot (the hang), and the next frame is whole.
        assert!(r.g.sched.band().is_none());
        let f = moved(&mut r, c, tex, 20, WHITE);
        assert_eq!(r.frame(&f), Step::Shown, "2D {two_d}");
        both_hold(&r, 20, WHITE, "after the hang");
    }
}
