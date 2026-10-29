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
    let mut rig = Rig {
        // Leaked: a test's renderer lives as long as the test process.
        g: Graphics::new(
            std::boxed::Box::leak(std::boxed::Box::new(two)),
            Pool::new(POOL),
            Some(std::boxed::Box::leak(std::boxed::Box::new(ThreeD::new()))),
        ),
        mem,
        clock,
    };
    // The models copy in no time, so no copy is ever timed here; on the
    // X51v each takes milliseconds. A copy's time known, as after the
    // device's first large copy (tests of the hold clear it).
    copies_timed(&mut rig);
    rig
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
    copies_timed(&mut r);
    let mut seen_behind = 0;
    for k in 0..12 {
        let f = moved(&mut r, c, tex, 4 + 2 * k, WHITE);
        let before = r.g.starts();
        let plan = r.g.sched.window(&scan::X51V_60HZ, true, band());
        assert_eq!(r.frame(&f), Step::Shown, "frame {k}");
        let after = r.g.starts();
        if let Some((from, to)) = plan {
            let line = after.last_line.unwrap();
            // The device run's log (R110): the window, the start, the time.
            assert_eq!(after.last_window, Some((from, to)), "frame {k}");
            assert!(after.last_start_us.is_some() && after.last_render_us.is_some());
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
            // No render history yet: the copy fallback (a copy timed).
            assert_eq!(after.screen1, before.screen1 + 1, "frame {k}");
        }
        assert_eq!(r.shown().at, SCREENS[0].at, "frame {k}: screen 0");
        both_hold(&r, 4 + 2 * k, WHITE, "frame");
    }
    assert!(seen_behind >= 10, "{seen_behind}");
    assert_eq!(r.g.starts().screen1, 1, "the first frame only");
    assert_eq!(r.g.starts().held, 0);
}

#[test]
fn the_copy_is_on_and_option_1_and_the_blanking_start_are_gone() {
    // The owner's ruling 2026-10-29 after GC2DF on the X51v: the copy on;
    // a frame the scan can't lead and no copy can place is held, never
    // started on screen 0 where it would tear.
    let mut r = rig();
    assert!(r.g.screen1_3d, "the copy on by default");
    let _ = scene(&mut r);
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    copies_timed(&mut r);
    assert!(matches!(
        r.g.plan_3d(true, &[]),
        Some(Start::Screen1 { .. })
    ));
    r.g.allow_screen1_3d(false);
    assert_eq!(r.g.plan_3d(true, &[]), Some(Start::Hold));
    r.g.allow_screen1_3d(true);
    r.g.copies.reset();
    assert_eq!(r.g.plan_3d(true, &[]), Some(Start::Hold));
}

#[test]
fn with_the_copy_off_unplaceable_frames_are_held_and_2d_still_shows() {
    // A checkerboard order (the divide-1 cure) never starts behind the
    // scan; with the copy off (a clock whose soak hasn't passed), every
    // such frame is held: 2D's changes shown, the scene not rendered.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    r.g.allow_screen1_3d(false);
    for k in 0..4u32 {
        let colour = [WHITE, 0xFF00_FF00][k as usize % 2];
        let f = moved(&mut r, c, tex, 8 + 4 * k as i32, colour);
        assert_eq!(r.frame(&f), Step::Shown, "frame {k}");
        assert_eq!(r.shown().at, SCREENS[0].at, "frame {k}");
        assert_eq!(r.on_screen(5, 5), colour & 0xFF_FFFF, "frame {k}: 2D shown");
        assert_ne!(
            r.on_screen(232, 336),
            0xFF_0000,
            "frame {k}: no scene drawn"
        );
    }
    let st = r.g.starts();
    assert_eq!((st.behind, st.held, st.screen1), (0, 4, 0));
    assert_eq!(st.last_line, None, "nothing started on screen 0");
}

/// GC2DE's whole-screen copy at 149.50 / 2 (3,149 us), as the copies'
/// history.
fn copies_timed(r: &mut Rig) {
    r.g.copies.reset();
    r.g.copies.record(3_149);
}

#[test]
fn once_screen_1_is_proven_unplaceable_frames_render_there_and_are_copied_behind_the_scan() {
    // The owner's ruling 2026-10-29 ("Yes, copy variant"): the frame
    // renders into screen 1 with screen 0 shown, then the area (and 2D's
    // changes held in screen 1) is copied into screen 0 at a line inside
    // the copy's window. Screen 0 is shown throughout: nothing flips.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    r.g.allow_screen1_3d(true);
    copies_timed(&mut r);
    for (k, two_d) in [(0, true), (1, true), (2, false), (3, true), (4, false)] {
        let colour = if k % 2 == 0 { WHITE } else { 0xFF00_FF00 };
        let mut f = moved(&mut r, c, tex, 8 + 4 * k, colour);
        if !two_d {
            f = still(0xFF00_FF00);
        }
        let mut step = r.g.frame(&f, &mut r.clock).unwrap();
        let (mut aside_seen, mut window) = (false, None);
        for _ in 0..10_000 {
            assert_eq!(r.shown().at, SCREENS[0].at, "frame {k}: screen 0 shown");
            if let Job::Aside { from, to, .. } = r.g.job {
                aside_seen = true;
                window = Some((from, to));
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
        let (from, to) = window.expect("copied");
        let line = r.g.starts().last_copy_line.unwrap();
        assert!(
            (from..=to).contains(&line),
            "frame {k}: {line} in {from}-{to}"
        );
        assert_eq!(r.shown().at, SCREENS[0].at, "frame {k}: never flipped");
        assert!(r.g.two_d().landed().is_none(), "frame {k}: nothing held");
        let bar = if two_d { colour } else { 0xFF00_FF00 };
        both_hold(&r, 8 + 4 * k, bar, &format!("copied frame {k}"));
    }
    let st = r.g.starts();
    assert_eq!((st.behind, st.held, st.screen1), (0, 0, 5));
    assert_eq!(r.g.copy_stalls(), 0);
}

#[test]
fn the_copy_waits_for_its_window_and_lands_behind_the_scan() {
    // A whole-screen scene (the copy's window from GC2DE's time: lines
    // 557-637): however the render ends, the copy starts inside the
    // window, and a copy of that time from that line is tear-free.
    let mut r = rig();
    let c = r.g.client(64 * 1024).unwrap();
    r.g.open_scene(c, 0, 0, 480, 640).unwrap();
    r.g.set_background(c, Some(BLUE)).unwrap();
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    r.g.allow_screen1_3d(true);
    copies_timed(&mut r);
    let area = [(0, 640, 480 * 640)];
    let (from, to) =
        scan::copy_window(&scan::X51V_60HZ, r.g.copies.band().unwrap(), &area).unwrap();
    for k in 0..6u32 {
        let colour = [WHITE, BLUE][k as usize % 2];
        r.g.set_background(c, Some(colour)).unwrap();
        // Frames asked for at lines spread over the refresh.
        r.clock.set(r.clock.0.get() + 2_731 * k);
        assert_eq!(r.frame(&still(WHITE)), Step::Shown, "frame {k}");
        let line = r.g.starts().last_copy_line.unwrap();
        assert!((from..=to).contains(&line), "frame {k}: {line}");
        assert!(scan::tear_free_rows(
            &scan::X51V_60HZ,
            (0, 640),
            line,
            3_149
        ));
        assert_eq!(r.on_screen(240, 320), colour & 0xFF_FFFF, "frame {k}");
    }
    assert_eq!(r.g.starts().screen1, 6);
}

#[test]
fn before_any_copy_is_timed_frames_are_held_then_drawn() {
    // The copy on, but no copy's time known yet: the frame is held (2D
    // shown, the scene still dirty); once a copy is timed the next frame
    // draws it.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    r.g.copies.reset();
    let f = moved(&mut r, c, tex, 8, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    let st = r.g.starts();
    assert_eq!((st.held, st.screen1), (1, 0));
    assert_eq!(r.on_screen(5, 5), WHITE & 0xFF_FFFF, "2D shown");
    assert_ne!(scene_r(&mut r).dirty(), 0, "the scene still to draw");
    copies_timed(&mut r);
    let f = moved(&mut r, c, tex, 12, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    assert_eq!(r.g.starts().screen1, 1);
    both_hold(&r, 12, WHITE, "after the hold");
}

#[test]
fn a_held_frame_keeps_the_scenes_last_picture_where_2d_drew_over_it() {
    // A frame drawn (the copy), then one held while 2D's frame paints a
    // green band across the area: the band shows outside the area, the
    // area keeps the scene's last picture (blue, the quad), never 2D's
    // layer under it. (2D's frame into screen 0 while screen 1 is shown:
    // `after_a_stall_2d_into_screen_0_is_shown_with_the_scene_drawn_again`.)
    for k in 0..1 {
        let mut r = rig();
        let (c, tex) = scene(&mut r);
        scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
        copies_timed(&mut r);
        let f = moved(&mut r, c, tex, 8, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
        both_hold(&r, 8, WHITE, "drawn");
        if k == 1 {
            // A 2D-only change (outside the area) flips to screen 1: 2D's
            // next frame goes into screen 0, not shown.
            // (2D's first frames are drawn whole, over the area too.)
            for _ in 0..3 {
                if r.shown().at == SCREENS[1].at {
                    break;
                }
                assert_eq!(
                    r.frame(&fill(Rect::new(0, 600, 480, 40), WHITE)),
                    Step::Shown
                );
            }
            assert_eq!(r.shown().at, SCREENS[1].at, "screen 1 shown");
        }
        r.g.copies.reset();
        // The scene changes too (its quad moved), so its frame is held: 2D
        // under an opaque scene alone needs no render (R110 run 3).
        r.g.set(c, 0, Some(&Quad::rect(12, 16, 32, 32, Fill::texture(tex))))
            .unwrap();
        let band = Rect::new(0, 300, 480, 80);
        assert_eq!(r.frame(&scene_2d(true, Some(band))), Step::Shown);
        {
            assert_eq!(r.g.starts().held, 1, "case {k}");
            assert_eq!(r.shown().at, SCREENS[0].at, "case {k}");
            // 2D's band shown on every side of the area: left, right, above,
            // below (the area is 200-263 x 304-367).
            for (x, y) in [(100, 340), (400, 340), (232, 302), (232, 372)] {
                assert_eq!(r.on_screen(x, y), 0x00_FF00, "case {k}: ({x}, {y})");
            }
            assert_eq!(
                r.on_screen(100, 340),
                0x00_FF00,
                "the band outside the area"
            );
            assert_eq!(
                r.on_screen(210, 310),
                BLUE & 0xFF_FFFF,
                "the scene's background kept"
            );
            assert_eq!(r.on_screen(232, 336), 0xFF_0000, "the scene's quad kept");
        }
    }
}

/// Tests: the copy takes 1 ms on the clock.
fn copy_takes_a_ms(two: &mut Mbx2dRenderer<Both, m2::Mem>) {
    let now = &two.parts().0.regs.now;
    now.set(now.get() + 1_000);
}

#[test]
fn copies_are_timed_per_whole_screen_and_small_ones_left_out() {
    let mut r = rig();
    let [safe, other] = r.g.two_d().screens();
    r.g.copies.reset();
    // A quarter of the screen in 1 ms: 4 ms a whole screen.
    r.g.before_copy = Some((1, copy_takes_a_ms));
    assert!(r.g.copy_screens(other, safe, &[Rect::new(0, 0, 480, 160)], &mut r.clock));
    assert_eq!(r.g.copies.band().map(|b| b.0), Some(3_200), "4 ms x 4/5");
    // A small one (under an eighth) is mostly the blit's own cost: left out.
    r.g.before_copy = Some((1, copy_takes_a_ms));
    assert!(r.g.copy_screens(other, safe, &[Rect::new(0, 0, 480, 32)], &mut r.clock));
    r.g.before_copy = Some((1, copy_takes_a_ms));
    assert!(r.g.copy_screens(other, safe, &[Rect::new(0, 0, 480, 80)], &mut r.clock));
    let (fast, slow) = r.g.copies.band().unwrap();
    assert_eq!(
        (fast, slow),
        (3_200, 16_000),
        "4 ms and the eighth's 8 ms kept (4/5 of the fastest, twice the last); the small one (15 ms) not"
    );
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
        r.g.plan_3d(true, &[]),
        Some(Start::Screen0 { .. })
    ));
    let (fast, slow) = r.g.sched.band().unwrap();
    assert!(slow >= 12_000, "{slow}");
    let (from, to) = r.g.sched.window(&scan::X51V_60HZ, true, band()).unwrap();
    for line in [from, to] {
        assert!(scan::tear_free_rows(&scan::X51V_60HZ, band(), line, 12_000));
        assert!(scan::tear_free_rows(&scan::X51V_60HZ, band(), line, fast));
    }
    r.g.sched.record(30_000);
    r.g.copies.reset();
    assert_eq!(r.g.plan_3d(true, &[]), Some(Start::Hold), "no copy timed");
    copies_timed(&mut r);
    assert!(matches!(
        r.g.plan_3d(true, &[]),
        Some(Start::Screen1 { .. })
    ));
    // A real frame after the spike: the copy fallback, then its own (fast)
    // time brings the scan back.
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
    assert_eq!(
        (st.behind, st.held),
        (0, 3),
        "held: never behind, no copy window"
    );
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
        copies_timed(&mut r);
        let f = moved(&mut r, c, tex, 8, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
        assert_eq!(r.g.starts().screen1, 1);
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

#[test]
fn the_copy_runs_down_the_screen_and_copies_nothing_twice() {
    let area = Rect::new(200, 304, 64, 64);
    let mut out = [Rect::default(); MAX_DIRTY + 1];
    // 2D's changes below and above the area: top first.
    let held = [Rect::new(0, 600, 480, 40), Rect::new(0, 0, 480, 32)];
    let n = copy_rects(&mut out, area, &held);
    assert_eq!(&out[..n], &[held[1], area, held[0]]);
    // A whole-screen 2D frame holds the area: copied once.
    let n = copy_rects(&mut out, area, &[Rect::new(0, 0, 480, 640)]);
    assert_eq!(&out[..n], &[Rect::new(0, 0, 480, 640)]);
    // A change inside the area, or the same rectangle twice: once.
    let n = copy_rects(&mut out, area, &[Rect::new(210, 310, 8, 8), area]);
    assert_eq!(&out[..n], &[area]);
}

#[test]
fn a_copy_is_timed_before_the_first_scene_frame_so_none_is_held() {
    // The owner's go 2026-10-29 (start-up copy timing, audit C94): a
    // service with a scene and no copy's time times one before its first
    // frame, so a frame the scan can't lead goes by the copy at once.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    scene_r(&mut r).set_tile_order(r3d::TileOrder::Checker);
    r.g.copies.reset();
    r.g.before_copy = Some((1, copy_takes_a_ms));
    let f = moved(&mut r, c, tex, 8, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    assert!(r.g.copy_times().is_some(), "a copy timed");
    let st = r.g.starts();
    assert_eq!((st.held, st.screen1), (0, 1));
    both_hold(&r, 8, WHITE, "the first frame");
}

#[test]
fn the_timing_copy_goes_from_the_shown_picture_and_changes_nothing_shown() {
    // Screen 1 shown (a 2D-only frame flipped there): the timing copy goes
    // into screen 0, not shown; what's shown doesn't change, and the hidden
    // picture then holds it too.
    let mut r = rig();
    for k in 0..3 {
        let colour = [WHITE, BLUE, 0xFF00_FF00][k];
        assert_eq!(
            r.frame(&fill(Rect::new(0, 0, 480, 40), colour)),
            Step::Shown
        );
    }
    let shown = r.shown();
    let before = (r.on_screen(5, 5), r.on_screen(240, 320));
    assert!(r.g.time_copy(&mut r.clock));
    assert_eq!(r.shown().at, shown.at, "nothing flipped");
    assert_eq!((r.on_screen(5, 5), r.on_screen(240, 320)), before);
    let hidden = SCREENS.iter().find(|s| s.at != shown.at).unwrap();
    let pic = Picture {
        at: hidden.at,
        ..Picture::screen(0)
    };
    assert_eq!(
        r.px(pic, 5, 5) & 0xFF_FFFF,
        before.0,
        "the hidden picture holds it too"
    );
    // Never with work in flight.
    r.g.job = Job::TwoD;
    assert!(!r.g.time_copy(&mut r.clock));
    r.g.job = Job::None;
}

#[test]
fn a_clock_change_forgets_both_times_and_the_next_frame_times_a_copy() {
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    let f = moved(&mut r, c, tex, 8, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    assert!(r.g.sched.band().is_some() && r.g.copy_times().is_some());
    r.g.clock_changed();
    assert!(r.g.sched.band().is_none(), "the renders' times forgotten");
    assert!(r.g.copy_times().is_none(), "the copies' times forgotten");
    r.g.before_copy = Some((1, copy_takes_a_ms));
    let f = moved(&mut r, c, tex, 12, WHITE);
    assert_eq!(r.frame(&f), Step::Shown);
    assert!(r.g.copy_times().is_some(), "timed again");
    assert_eq!(r.g.starts().held, 0);
    both_hold(&r, 12, WHITE, "after the clock change");
}

#[test]
fn only_the_scenes_owner_sets_its_tile_order_and_never_mid_frame() {
    let mut r = rig();
    let (c, _) = scene(&mut r);
    let other = r.g.client(1024).unwrap();
    assert_eq!(
        r.g.set_tile_order(other, r3d::TileOrder::Checker),
        Err(Error::NotYourScene)
    );
    assert_eq!(scene_r(&mut r).tile_order(), r3d::TileOrder::Rows);
    r.g.set_tile_order(c, r3d::TileOrder::Checker).unwrap();
    assert_eq!(scene_r(&mut r).tile_order(), r3d::TileOrder::Checker);
    // A frame in flight: refused, the order kept.
    r.g.job = Job::TwoD;
    assert_eq!(
        r.g.set_tile_order(c, r3d::TileOrder::Rows),
        Err(Error::Busy)
    );
    assert_eq!(scene_r(&mut r).tile_order(), r3d::TileOrder::Checker);
    r.g.job = Job::None;
}

#[test]
fn a_2d_change_beside_an_opaque_scene_goes_in_behind_the_scan_and_never_redraws_it() {
    // R110 run 3 on the X51v: 2D's changes copied into the shown screen as
    // they landed tore every frame and drew the scene again for nothing.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    copies_timed(&mut r);
    for k in 0..6 {
        let f = moved(&mut r, c, tex, 4 + 2 * k, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
    }
    for (i, colour) in [GREY, WHITE, BLUE].into_iter().enumerate() {
        let before = r.g.starts();
        let mut step =
            r.g.frame(&fill(Rect::new(0, 0, 480, 32), colour), &mut r.clock)
                .unwrap();
        let mut waited = None;
        for _ in 0..10_000 {
            if let Job::Copy2d { from, to, .. } = r.g.job {
                waited = Some((from, to));
            }
            match step {
                Step::Wait(t) => {
                    r.clock.set(t);
                    step = r.g.poll(&mut r.clock);
                }
                _ => break,
            }
        }
        assert_eq!(step, Step::Shown, "frame {i}");
        let a = r.g.starts();
        // Copied at a line inside its window (at once when the scan was
        // already inside it).
        let line = a.last_2d_line.unwrap();
        if let Some((from, to)) = waited {
            assert!(
                (from..=to).contains(&line),
                "frame {i}: {line} in {from}-{to}"
            );
        }
        assert_eq!(a.two_d_behind, before.two_d_behind + 1, "frame {i}");
        assert_eq!(a.two_d_now, before.two_d_now);
        assert_eq!(
            (a.behind, a.screen1, a.held),
            (before.behind, before.screen1, before.held),
            "frame {i}: the scene not drawn again"
        );
        assert_eq!(r.shown().at, SCREENS[0].at, "frame {i}: screen 0 shown");
        both_hold(&r, 14, colour, &format!("2D frame {i}"));
    }
}

#[test]
fn two_d_drawn_under_an_opaque_scene_never_reaches_the_shown_area() {
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    copies_timed(&mut r);
    for k in 0..6 {
        let f = moved(&mut r, c, tex, 4 + 2 * k, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
    }
    // A green band across the area (2D's layer under the scene).
    let before = r.g.starts();
    let band = Rect::new(0, 300, 480, 80);
    assert_eq!(r.frame(&scene_2d(true, Some(band))), Step::Shown);
    let a = r.g.starts();
    assert_eq!(
        (a.behind, a.screen1),
        (before.behind, before.screen1),
        "no render"
    );
    assert_eq!(r.on_screen(100, 340), 0x00_FF00, "the band beside the area");
    // The area keeps the scene in both screens (screen 1's made current).
    both_hold(&r, 14, WHITE, "after the band");
}

#[test]
fn two_d_under_a_translucent_scene_draws_the_scene_again() {
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    r.g.set_background(c, None).unwrap();
    copies_timed(&mut r);
    for k in 0..6 {
        let f = moved(&mut r, c, tex, 4 + 2 * k, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
    }
    let before = r.g.starts();
    let band = Rect::new(0, 300, 480, 80);
    assert_eq!(r.frame(&scene_2d(true, Some(band))), Step::Shown);
    let a = r.g.starts();
    assert_eq!(
        a.behind + a.screen1 + a.held,
        before.behind + before.screen1 + before.held + 1,
        "2D under it shows through: the scene drawn again"
    );
}

#[test]
fn after_a_stall_2d_into_screen_0_is_shown_with_the_scene_drawn_again() {
    // The one way screen 1 is shown with a scene open: a stalled copy of
    // 2D's changes. 2D's next frame goes into screen 0 (not shown): the
    // area is put back from screen 1 before screen 0 is shown, and the
    // scene, invalidated by the stall, is drawn again.
    let mut r = rig();
    let (c, tex) = scene(&mut r);
    copies_timed(&mut r);
    for k in 0..6 {
        let f = moved(&mut r, c, tex, 4 + 2 * k, WHITE);
        assert_eq!(r.frame(&f), Step::Shown);
    }
    r.g.before_copy = Some((1, stall));
    assert_eq!(
        r.frame(&fill(Rect::new(0, 0, 480, 32), GREY)),
        Step::Recovered
    );
    assert_eq!(
        r.shown().at,
        SCREENS[1].at,
        "screen 1 shown after the stall"
    );
    assert_eq!(r.g.copy_stalls(), 1);
    r.g.two_d().parts().0.regs.two.full_reads = 0;
    let before = r.g.starts();
    let mut step =
        r.g.frame(&fill(Rect::new(0, 0, 480, 32), WHITE), &mut r.clock)
            .unwrap();
    for _ in 0..10_000 {
        match step {
            Step::Wait(t) => {
                r.clock.set(t);
                step = r.g.poll(&mut r.clock);
            }
            _ => break,
        }
    }
    let a = r.g.starts();
    assert_eq!(r.shown().at, SCREENS[0].at, "screen 0 shown again");
    assert_eq!(
        a.behind + a.screen1,
        before.behind + before.screen1 + 1,
        "drawn again"
    );
    // On screen: the scene and 2D's bar (screen 1 catches 2D up on 2D's
    // next frame, as ever).
    let qx = u32::from(AREA.0) + 14 + 16;
    assert_eq!(r.on_screen(qx, 336), 0xFF_0000, "the quad");
    assert_eq!(r.on_screen(210, 310), BLUE & 0xFF_FFFF, "the background");
    assert_eq!(r.on_screen(5, 5), WHITE & 0xFF_FFFF, "2D's bar");
}
