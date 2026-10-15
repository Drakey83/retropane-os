//! Test image (ADR-0024 M4): the first triangle drawn by the 2700G's MBX
//! 3D core, under the loader.
//!
//! Everything it sends is what Windows Mobile's own driver sends (docs/hw/
//! 2700g.md, "The scene bytes"; encoded by `rp-drv-mbx3d`): a region array
//! over a tile grid, every tile pointing at one object list (a strip
//! pointer, then the end word), one 17-word triangle (ISP "always", Dell's
//! TSP word, x/y pairs, z, 1/w, colours), then Dell's 15-pair start table,
//! the target a 32 bpp picture in 2700G memory. Three things are unknown and
//! are found here:
//!
//! - **whether it draws at all, and how it says it's done:** after the
//!   start, `0x12c` (status), `0x804` (noted 2026-10-13) and a pixel the
//!   triangle covers are polled for 200 ms, every change logged with its
//!   time;
//! - **the tile size:** a screen-covering triangle (Dell's background, red)
//!   is drawn with a grid that assumes 32-pixel tiles (15 x 20); how far
//!   the red reaches says the real size (16-pixel tiles: the top-left
//!   quarter; 64: all of it, from half as many tiles);
//! - **whether `0x654`/`0x658` count pixels or tiles:** pixels first (as
//!   Dell's table has them); if nothing is drawn, tiles.
//!
//! Then, if the red background drew: a triangle with red, green and blue
//! corners over grey (shading), read back at its corners and middle, and
//! shown until a button. Every wait has a timeout; the core is reset
//! after each try and at the end; the plane and every register changed is
//! put back (the 2700G keeps state between images). Only registers Dell's
//! driver touches are read (a wrong read hangs the bus).

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-MBX3D";

/// The tile size the red background's extent implies: `reach` pixels were
/// drawn along an axis covered by `tiles` tiles of the assumed size. None
/// if nothing (or an odd amount) was drawn.
#[must_use]
pub fn tile_size(reach: u32, tiles: u32, screen: u32) -> Option<u32> {
    if reach == 0 || tiles == 0 {
        return None;
    }
    if reach >= screen {
        // All of it: the tiles are at least the assumed size.
        return Some(screen.div_ceil(tiles).next_power_of_two());
    }
    let size = reach / tiles;
    (size.is_power_of_two() && size * tiles == reach).then_some(size)
}

#[cfg(test)]
mod tests {
    use super::tile_size;

    #[test]
    fn tile_size_from_the_red_reach() {
        // 15 x 20 tiles assumed (32 px); 16-pixel tiles draw 240 x 320.
        assert_eq!(tile_size(240, 15, 480), Some(16));
        assert_eq!(tile_size(320, 20, 640), Some(16));
        assert_eq!(tile_size(480, 15, 480), Some(32));
        assert_eq!(tile_size(0, 15, 480), None);
        assert_eq!(tile_size(250, 15, 480), None, "not whole tiles");
    }
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, layers::Layer, reg as dreg};
    use rp_drv_mbx3d::{self as m3, ISP_ALWAYS, TSP_DELL, Vertex};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{MARKER, tile_size};

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// Microseconds since `t0` (OSCR0 at 3.25 MHz).
    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    // 2700G local memory: the 32 bpp target (2 MiB in, clear of the
    // loader's plane below 1 MiB), the parameter buffer (4 KiB aligned:
    // the object list at +0, the triangle at +64, as Dell's minimal scene),
    // the region array, the 2D engine's fence.
    const TARGET: usize = 0x0020_0000;
    const W: u32 = 480;
    const H: u32 = 640;
    const STRIDE: usize = 480 * 4;
    const PARAMS: usize = 0x00C0_0000;
    const OBJECT: usize = PARAMS + 64;
    const REGIONS: usize = 0x00C1_0000;
    const FENCE: usize = 0x00FF_F000;
    /// The tile size assumed for the grid.
    const TILE: u32 = 32;
    const COLS: u32 = W / TILE;
    const ROWS: u32 = H / TILE;

    /// MBX registers read here (all ones Dell's driver touches).
    const STATUS: usize = MBX + reg::STATUS;
    const R804: usize = MBX + 0x804;

    const GREY: u32 = 0xFF20_2020;

    fn store(vram: &mut [u8], at: usize, words: &[u32]) {
        for (k, &w) in words.iter().enumerate() {
            let _ = store32_volatile(vram, at + 4 * k, w);
        }
    }

    fn px(vram: &[u8], x: u32, y: u32) -> u32 {
        load32_volatile(vram, TARGET + y as usize * STRIDE + 4 * x as usize).unwrap_or(0xDEAD_BEEF)
    }

    /// Clears the target to `c` with the 2D engine (proven), waits for it.
    fn clear(e: &mut Engine<Mmio>, vram: &[u8], c: u32) -> bool {
        let w = cmd::fill(Format::BPP32, TARGET, STRIDE, (0, 0, W as u16, H as u16), c);
        e.submit(&w).and_then(|()| e.finish(vram, 200_000)).is_ok()
    }

    /// How a try's start table sizes the target.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Size {
        /// `0x654`/`0x658` = (pixels - 1) << 16, as Dell's table.
        Pixels,
        /// (tiles - 1) << 16.
        Tiles,
    }

    /// Writes one triangle's scene and starts the 3D stage, then polls the
    /// status, `0x804` and `probe` for 200 ms. Returns whether `probe`
    /// changed (the triangle drew), logging each change with its time.
    fn draw(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        v: &[Vertex; 3],
        size: Size,
        probe: (u32, u32),
    ) -> bool {
        let object = m3::triangle(ISP_ALWAYS, TSP_DELL, v);
        store(vram, OBJECT, &object);
        store(
            vram,
            PARAMS,
            &m3::one_triangle_list((OBJECT - PARAMS) as u32),
        );
        let mut regions = [0u32; 2 * 15 * 20];
        let Some(n) = m3::region_array(&mut regions, COLS, ROWS, PARAMS as u32) else {
            log!("  region array doesn't fit");
            return false;
        };
        store(vram, REGIONS, &regions[..n]);
        cpu::drain_write_buffer();
        let (w, h) = match size {
            Size::Pixels => (W, H),
            Size::Tiles => (COLS, ROWS),
        };
        let table = m3::start_table(REGIONS as u32, PARAMS as u32, w, h, STRIDE as u32);
        let before = px(vram, probe.0, probe.1);
        let (s0, r0) = (e.regs.read32(STATUS), e.regs.read32(R804));
        log!("  before: status 0x{s0:08x}, 0x804 0x{r0:08x}, probe pixel 0x{before:08x}");
        let t0 = rt::ticks();
        for (reg_off, value) in table {
            e.regs.write32(MBX + reg_off, value);
        }
        let (mut s, mut r, mut p) = (s0, r0, before);
        let mut changes = 0;
        let mut drew = None;
        while us_since(t0) < 200_000 {
            let now = us_since(t0);
            let (s1, r1, p1) = (
                e.regs.read32(STATUS),
                e.regs.read32(R804),
                px(vram, probe.0, probe.1),
            );
            if (s1, r1, p1) != (s, r, p) && changes < 12 {
                changes += 1;
                log!("  {now} us: status 0x{s1:08x}, 0x804 0x{r1:08x}, probe 0x{p1:08x}");
            }
            if p1 != before && drew.is_none() {
                drew = Some(now);
            }
            (s, r, p) = (s1, r1, p1);
        }
        log!(
            "  after 200 ms: status 0x{s:08x}, 0x804 0x{r:08x}; probe {}",
            match drew {
                Some(t) => {
                    let _ = writeln!(RunLog, "  probe changed at {t} us");
                    "DREW"
                }
                None => "unchanged",
            }
        );
        drew.is_some()
    }

    /// How far a colour reaches from the top-left along row `y` and column
    /// `x` (pixels whose colour differs from `bg`, stepping by 2).
    fn reach(vram: &[u8], bg: u32) -> (u32, u32) {
        let across = (0..W)
            .step_by(2)
            .take_while(|&x| px(vram, x, 2) != bg)
            .count() as u32
            * 2;
        let down = (0..H)
            .step_by(2)
            .take_while(|&y| px(vram, 2, y) != bg)
            .count() as u32
            * 2;
        (across, down)
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(vram.len());
        let regs = display.into_inner();
        let mut e = Engine::new(regs, FENCE, rt::ticks, 3_250);
        log!("{MARKER}: the MBX 3D core's first triangle (ADR-0024 M4)");
        log!(
            "core ID 0x{:08x}, status 0x{:08x}, 0x804 0x{:08x}, 0x85c 0x{:08x}",
            e.regs.read32(MBX + reg::ID),
            e.regs.read32(STATUS),
            e.regs.read32(R804),
            e.regs.read32(MBX + 0x85c)
        );
        e.reset(rt::delay_us);
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();

        // A: the screen-covering red triangle (Dell's background), pixels
        // then tiles in the size registers.
        let background = [
            Vertex::flat(0.0, 0.0, 0xFFFF_0000),
            Vertex::flat(2048.0, 0.0, 0xFFFF_0000),
            Vertex::flat(0.0, 2048.0, 0xFFFF_0000),
        ];
        let mut drew = false;
        let mut size_used = Size::Pixels;
        for (name, size) in [("pixels", Size::Pixels), ("tiles", Size::Tiles)] {
            rt::check_in();
            if !clear(&mut e, vram, GREY) {
                log!("A: the 2D clear didn't finish: stopping");
                break;
            }
            log!(
                "A: red screen-covering triangle, {COLS} x {ROWS} grid of {TILE} px, sizes in {name}"
            );
            drew = draw(&mut e, vram, &background, size, (8, 8));
            e.reset(rt::delay_us);
            if drew {
                size_used = size;
                let (across, down) = reach(vram, GREY);
                log!(
                    "A: drawn {across} px across, {down} px down; pixel (8,8) 0x{:08x}, (240,320) 0x{:08x}, (470,630) 0x{:08x}",
                    px(vram, 8, 8),
                    px(vram, 240, 320),
                    px(vram, 470, 630)
                );
                log!(
                    "A: tile size from the reach: across {:?}, down {:?}",
                    tile_size(across, COLS, W),
                    tile_size(down, ROWS, H)
                );
                break;
            }
        }

        // B: red, green and blue corners over grey (shading), if A drew.
        let mut shown = false;
        if drew {
            rt::check_in();
            let _ = clear(&mut e, vram, GREY);
            let tri = [
                Vertex::flat(240.0, 80.0, 0xFFFF_0000),
                Vertex::flat(420.0, 560.0, 0xFF00_FF00),
                Vertex::flat(60.0, 560.0, 0xFF00_00FF),
            ];
            log!("B: red/green/blue triangle (240,80) (420,560) (60,560)");
            let ok = draw(&mut e, vram, &tri, size_used, (240, 400));
            e.reset(rt::delay_us);
            log!(
                "B: {}; near the corners 0x{:08x} 0x{:08x} 0x{:08x}, middle 0x{:08x}, outside 0x{:08x}",
                if ok { "drew" } else { "DIDN'T DRAW" },
                px(vram, 240, 90),
                px(vram, 405, 552),
                px(vram, 75, 552),
                px(vram, 240, 400),
                px(vram, 20, 20)
            );
            shown = ok;
        }

        // Show the target (32 bpp, opaque) until a button; plane put back.
        let saved = show_target(&mut e.regs);
        let keypad = arm_keypad();
        let verdict = if drew {
            "DREW"
        } else {
            "nothing drawn (see the log)"
        };
        log!(
            "{MARKER}: {verdict} - {} until a button",
            if shown {
                "the shaded triangle is"
            } else {
                "the target is"
            }
        );
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: {verdict}");
        }
        hold_until_button(keypad);
        for (off, v) in saved {
            e.regs.write32(off, v);
        }
        e.reset(rt::delay_us);
        rp_hal_x51v::devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Points the graphics plane at the 32 bpp target (opaque); returns the
    /// registers as found.
    fn show_target(regs: &mut Mmio) -> [(usize, u32); 5] {
        let plane = Plane {
            format: PixelFormat::Argb8888,
            width: W as u16,
            height: H as u16,
            offset: TARGET,
            stride: STRIDE,
        };
        let new = plane.registers();
        let saved = new.map(|(off, _)| (off, regs.read32(off)));
        let gamma = regs.read32(dreg::GSCTRL) & (1 << 26);
        for (off, v) in new {
            let v = match off {
                dreg::GSCTRL => v | gamma,
                dreg::GSCADR => Layer::SOLO.scadr(v),
                dreg::GBBASE => Layer::SOLO.bbase(v),
                _ => v,
            };
            regs.write32(off, v);
        }
        saved
    }

    /// The keypad, enabled with earlier activity discarded, before the
    /// verdict goes out (a press right after it must count).
    fn arm_keypad() -> Option<Keypad<Mmio>> {
        let kp = rt::take_keypad()?;
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        Some(keypad)
    }

    /// Holds until a button is pressed and released (5 minutes at most),
    /// checking in with the loader.
    fn hold_until_button(keypad: Option<Keypad<Mmio>>) {
        let Some(mut keypad) = keypad else {
            return;
        };
        let t0 = rt::ticks();
        let mut seen = false;
        while us_since(t0) < 300_000_000 {
            rt::check_in();
            seen |= keypad.acknowledge();
            if seen && keypad.pressed() == Pressed::NONE {
                return;
            }
            rt::delay_us(20_000);
        }
        log!("no button in 5 minutes: returning");
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        rp_hal_x51v::devices::crash_to_loader(msg);
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        crash(format_args!("PANIC {}", info.message()))
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
