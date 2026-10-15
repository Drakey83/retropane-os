//! Test image (ADR-0024 M4): the first triangle drawn by the 2700G's MBX
//! 3D core, under the loader. Version 16: version 15 drew the GLES clear
//! through the TA (black over grey; the TA's flat object: TSP, x/y x3,
//! z x3). Now the clear again (control), then a textured triangle through
//! the TA, as the GLES driver sends one: the state block with texture
//! unit 0's words (mask `0x10000057`), a strip word, three 7-word vertices
//! (x, y, z, 1/w, colour, u, v), with ISP "always" and with the game's
//! ISP; the TA's object logged word for word.
//! Version 15: version 14's TA ran (done in
//! 1.9 ms, a region array in the game's form) but wrote its lists 4 MiB
//! low (`0xEDC6DC` as `0x6DC6DC`): the parameter memory manager reaches
//! only 8 MiB (11-bit page numbers), so the parameter memory moves from
//! 12 MiB to 6 MiB; the words around the first list are logged too.
//! Version 14 (ADR-0024 M5): hand-written
//! textured objects never drew with every register as the game has them
//! (versions 7-13), so this draws the way the game does, through the tile
//! accelerator: the GLES driver's own clear, word for word, streamed into
//! the TA's window (`0x0FFF4000`), the end word into `0x0FFF6000`, the TA
//! started with the GLES driver's table; what the TA wrote is logged, then
//! the 3D stage renders the TA's region array. The texture tries are gone.
//! The scene ends as the GLES driver's flush ends it: `0x85C` bit 7 set,
//! `0xF0000000` into the TA window, then a wait for TA done (bit 4).
//! Version 13: version 12's frame state drew no
//! textured object either. One register the game renders with is left:
//! `0x6D8`. The GLES driver never writes it, so in the game it keeps the
//! parameter memory set-up's `0x09000000` (bit 27); Dell's start table,
//! which ours copies, writes `0x01000000` (Dell's own scenes are flat).
//! The flat control, the game's object and our square with it, alone and
//! with version 12's levers, then the game's object without (control).
//! Version 12: version 11's `0x61C` changed
//! nothing. Read since (static, 2026-10-15): DoomGLES's frames set bit 29
//! on every region header (ours never), the GLES driver writes `0x6DC` =
//! the clear depth as a float with bit 0 (`0x3F800001`; ours 0 or 1, a
//! clear depth near 0), a depth buffer at `0x674` with
//! `0x610 = (tiles - 1) << 7 | 2`, and `0x860` = the tile grid. The game's object and our
//! square with each of those, alone and together, after a flat control
//! with all of them; the depth buffer's first word is checked for a store.
//! Version 11: version 10 showed DoomGLES's
//! own object doesn't draw either: textures need state outside the object.
//! The GLES driver's 3D table sets `0x61C = 0x00020007` where Dell's has
//! `0x00020004`: the same tries with that, then with the GLES table's
//! other values too. Version 10: version 9's flat control
//! drew (a list draws past its first object; the flags must match the
//! object: 0x41000000 dropped a flat one), its textured square never: now
//! DoomGLES's own first object, word for word, only its TEX1 pointed at
//! our texture, with each flag its lists used. Version 9: version 8 drew the dark
//! background but never the square: first two controls (the square flat,
//! after the background: does a list draw past its first object?), then
//! the textured forms with Dell's table and with the GLES table's extras.
//! Version 8: version 7's square never showed
//! (every sample the core's white tile fill, drawn or not: a flaw of the
//! test), so first a dark background and a tinted square in 8 forms (ISP
//! word, corner order for culling, list flags), then the format sweep with
//! the first form that draws. Version 7: after the shaded triangle (TSP
//! bit 22, Gouraud, proven by version 6), a textured square in DoomGLES's
//! object form (ADR-0024 M3, capture version 6), once for each of the 16
//! values of the texture format field: the texture holds its own texel
//! numbers (texel k = k, 16 bits), so the pixels logged tell, on the PC,
//! the format, the texel order and the size code. Unattended: it returns
//! by itself after 10 s.
//!
//! Version 4 drew the whole screen (Dell's table alone, 8 x 16 px regions) but
//! the shaded triangle flat; version 5 tried ISP bit 23.
//!
//! Version 3 drew (2026-10-14): the 3D clock was the missing piece; each
//! region entry covered 8 x 16 pixels. Version 4 sizes the grid to that
//! (60 x 40, written straight into 2700G memory), learns whether `0x65C`
//! is needed (Dell's table alone first), and draws the shaded triangle
//! inside the grid. Version 2's notes follow.
//!
//! Version 1 (2026-10-14) sent only Dell's 15-pair start table: nothing
//! drew, nothing moved, nothing hung. Dell's driver does three things first
//! that it skipped (docs/hw/2700g.md, "The scene bytes"):
//!
//! 1. **the 3D core's clock on**: `MBXCLK` (system block `0x054`) bits 1-0
//!    are an enable (Linux mbxfb: 0 none, 1 2D only, 2 2D and 3D);
//!    Windows Mobile leaves `0x5` (2D only) and Dell's submit routine
//!    writes `((divider - 1) << 2) | 2` before 3D work. Version 2 read it
//!    as a clock source and its check misread `PLLSTAT 0x5` (both PLLs
//!    locked) as a PLL off: the 3D clock was never on. Tried as found
//!    first (the control), then with 3D on;
//! 2. **the parameter memory manager**: six pairs (`0x824`-`0x83C`, `0x6D8`,
//!    then `0x838 = 1`) with status bit 6 enabled in `0x130`, then a wait
//!    for that bit in `0x12c` ("ready"), cleared through `0x134`;
//! 3. **status bits enabled before polling them**: Dell sets the bit in
//!    `0x130`, polls `0x12c`, clears through `0x134`.
//!
//! Then, if the parameter memory answers: the screen-covering red triangle
//! (Dell's background) on an assumed 32-px tile grid, its lists inside the
//! parameter memory as Dell keeps them, with render-done (bit 2) enabled:
//! try A Dell's table plus the likely target register `0x65C`; try B
//! adding the GLES table's extras. How far the red reaches gives the tile
//! size. Then a red/green/blue triangle, read back and shown until a
//! button. Every wait has a timeout; the core is reset at the end; the
//! clock, the plane and every register changed are put back. Only
//! registers Dell's driver touches are read (a wrong read hangs the bus).

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

/// `MBXCLK` with the same divider and the 3D core's clock on too: bits
/// 1-0 are the enable (0 none, 1 2D only, 2 2D and 3D; Linux mbxfb's
/// `MBXCLK_EN`), bits 3-2 the divider. Windows Mobile leaves `0x5` (2D
/// only); Dell's driver writes 2 there before 3D work.
#[must_use]
pub const fn with_3d(mbxclk: u32) -> u32 {
    (mbxclk & !3) | 2
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, layers::Layer, reg as dreg};
    use rp_drv_mbx3d::{
        self as m3, ISP_ALWAYS, PM_READY, RENDER_DONE, TSP_DELL, TSP_GOURAUD, Vertex,
    };
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{MARKER, tile_size, with_3d};

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// Microseconds since `t0` (OSCR0 at 3.25 MHz).
    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    // 2700G local memory: the 32 bpp target (2 MiB in, clear of the
    // loader's plane below 1 MiB); the parameter memory (1 MiB at 6 MiB,
    // 4 KiB aligned, below the manager's 8 MiB reach (version 14); our
    // lists in its first 4 KiB, where Dell keeps its own); the manager's
    // 16 KiB block (16 KiB aligned); the region array;
    // the 2D engine's fence.
    const TARGET: usize = 0x0020_0000;
    const W: u32 = 480;
    const H: u32 = 640;
    const STRIDE: usize = 480 * 4;
    const PM: usize = 0x0060_0000;
    const PM_SIZE: u32 = 0x0010_0000;
    const PM_TABLE: usize = 0x0070_0000;
    const _: () = assert!(PM + PM_SIZE as usize <= m3::PM_REACH);
    const PARAMS: usize = PM;
    const OBJECT: usize = PARAMS + 64;
    const REGIONS: usize = 0x00D1_0000;
    const FENCE: usize = 0x00FF_F000;
    /// Stage X (the tile accelerator): its per-tile buffer (`0x840`,
    /// 128 KiB aligned, zeroed; Dell sizes it 4 bytes a tile in Morton
    /// order, under 16 KiB here) and the region array it writes.
    const TAIL: usize = 0x00A0_0000;
    const TAIL_SIZE: usize = 0x1_0000;
    const TA_REGIONS: usize = 0x00A2_0000;
    /// Stage X's texture: 256 x 256 16-bit texels (128 KiB), texel k
    /// holding k (any format decodes it to something visible).
    const TEXTURE: usize = 0x00E0_0000;
    /// The grid: one region entry per 8 x 16 pixels (measured, version 3).
    const COLS: u32 = W / m3::TILE_W;
    const ROWS: u32 = H / m3::TILE_H;

    /// MBX registers (all ones Dell's driver touches).
    const STATUS: usize = MBX + reg::STATUS;
    const ENABLE: usize = MBX + reg::ENABLE;
    const CLEAR: usize = MBX + reg::CLEAR;
    const R804: usize = MBX + 0x804;
    /// The 3D core's clock (system block, read fine under WM).
    /// This image's version, in its first log line (kept here so the
    /// banner can't fall behind the module notes again).
    const VERSION: u32 = 16;
    const MBXCLK: usize = 0x054;
    /// The PLLs' status (system block): bit 2 the core PLL locked, bit 0
    /// the display PLL locked (Linux mbxfb).
    const PLLSTAT: usize = 0x040;

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

    /// Writes `pairs` to the MBX block with `bit` enabled in `0x130`, then
    /// polls `0x12c`, `0x804` and (if given) a target pixel for `limit_us`,
    /// logging every change with its time. Returns (when the bit set, when
    /// the pixel changed). The bit is cleared through `0x134` and the
    /// enables put back.
    fn run(
        e: &mut Engine<Mmio>,
        vram: &[u8],
        pairs: &[(usize, u32)],
        bit: u32,
        probe: Option<(u32, u32)>,
        limit_us: u32,
    ) -> (Option<u32>, Option<u32>) {
        let en = e.regs.read32(ENABLE);
        e.regs.write32(CLEAR, bit);
        e.regs.write32(ENABLE, en | bit);
        let p0 = probe.map(|(x, y)| px(vram, x, y));
        let (s0, r0) = (e.regs.read32(STATUS), e.regs.read32(R804));
        log!(
            "  before: status 0x{s0:08x}, enable 0x{:08x}, 0x804 0x{r0:08x}, probe {p0:08x?}",
            en | bit
        );
        let t0 = rt::ticks();
        for &(off, v) in pairs {
            e.regs.write32(MBX + off, v);
        }
        let (mut s, mut r, mut p) = (s0, r0, p0);
        let (mut set, mut drew) = (None, None);
        let mut changes = 0;
        while us_since(t0) < limit_us {
            let now = us_since(t0);
            let s1 = e.regs.read32(STATUS);
            let r1 = e.regs.read32(R804);
            let p1 = probe.map(|(x, y)| px(vram, x, y));
            if (s1, r1, p1) != (s, r, p) && changes < 12 {
                changes += 1;
                log!("  {now} us: status 0x{s1:08x}, 0x804 0x{r1:08x}, probe {p1:08x?}");
            }
            if s1 & bit != 0 && set.is_none() {
                set = Some(now);
            }
            if p1 != p0 && drew.is_none() {
                drew = Some(now);
            }
            (s, r, p) = (s1, r1, p1);
            if set.is_some() && (probe.is_none() || drew.is_some()) && now > 2_000 {
                break;
            }
        }
        e.regs.write32(CLEAR, bit);
        e.regs.write32(ENABLE, en);
        log!(
            "  bit 0x{bit:x} set at {set:?} us, probe changed at {drew:?} us; status now 0x{:08x}",
            e.regs.read32(STATUS)
        );
        (set, drew)
    }

    /// The parameter memory manager's set-up; true if it said ready.
    fn pm_init(e: &mut Engine<Mmio>, vram: &mut [u8]) -> bool {
        for k in 0..4096 {
            let _ = store32_volatile(vram, PM_TABLE + 4 * k, 0);
        }
        cpu::drain_write_buffer();
        let t = m3::pm_init_table(PM as u32, PM_SIZE, PM_TABLE as u32);
        run(e, vram, &t, PM_READY, None, 100_000).0.is_some()
    }

    /// One triangle's scene inside the parameter memory, then `table`
    /// (start last), render-done enabled; true if `probe` changed.
    fn draw(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        tsp: u32,
        v: &[Vertex; 3],
        table: &[(usize, u32)],
        probe: (u32, u32),
    ) -> bool {
        store(vram, OBJECT, &m3::triangle(ISP_ALWAYS, tsp, v));
        store(
            vram,
            PARAMS,
            &m3::one_triangle_list((OBJECT - PARAMS) as u32),
        );
        regions(vram, 0);
        run(e, vram, table, RENDER_DONE, Some(probe), 200_000)
            .1
            .is_some()
    }

    /// The region array, every tile on the list at the parameter
    /// buffer's start, `flags` ORed into each header. Straight into 2700G
    /// memory (19 KiB: too big for the stack).
    #[inline(never)]
    fn regions(vram: &mut [u8], flags: u32) {
        let n = COLS * ROWS;
        for k in 0..n {
            let mut entry = m3::region_entry(k % COLS, k / COLS, PARAMS as u32, k + 1 == n);
            entry[0] |= flags;
            store(vram, REGIONS + 8 * k as usize, &entry);
        }
        cpu::drain_write_buffer();
    }

    /// Logs `n` words from 2700G memory at `at` (nothing if out of range).
    fn dump(vram: &[u8], what: &str, at: usize, n: usize) {
        let _ = write!(RunLog, "  {what} @0x{at:06x}:");
        for k in 0..n {
            match load32_volatile(vram, at + 4 * k) {
                Some(w) => {
                    let _ = write!(RunLog, " {w:08x}");
                }
                None => break,
            }
        }
        log!("");
    }

    /// Stage X: the TA's buffers zeroed, its start table, `stream` into
    /// the TA's window, `1` into the end window, then the scene's end as
    /// the GLES driver's flush sends it (`0x0226fdc0`: `0x85C` bit 7,
    /// `0xF0000000` into the TA window), all as one write list; wait for
    /// TA done (status bit 4) and log what the TA wrote (`words` of the
    /// first object); if it finished, the 3D stage on the TA's region
    /// array (Dell's table `level`). True if `probe` changed.
    #[inline(never)]
    fn through_ta(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        level: u8,
        stream: &[u32],
        words: usize,
        probe: (u32, u32),
    ) -> bool {
        for k in 0..TAIL_SIZE / 4 {
            let _ = store32_volatile(vram, TAIL + 4 * k, 0);
        }
        for k in 0..2 * (COLS * ROWS + 1) as usize {
            let _ = store32_volatile(vram, TA_REGIONS + 4 * k, 0);
        }
        cpu::drain_write_buffer();
        let t = m3::ta::start_table(TAIL as u32, TA_REGIONS as u32, W, H, COLS, ROWS);
        // `run` writes at the MBX block's offsets: the windows are below it.
        let n = stream.len().min(32);
        let mut w = [(0usize, 0u32); 9 + 32 + 3];
        w[..9].copy_from_slice(&t);
        for (k, &v) in stream[..n].iter().enumerate() {
            w[9 + k] = (m3::ta::PORT - MBX + 4 * k, v);
        }
        let r85c = t[7];
        w[9 + n] = (m3::ta::END_PORT - MBX, 1);
        w[10 + n] = (r85c.0, r85c.1 | m3::ta::SCENE_END);
        w[11 + n] = (m3::ta::PORT - MBX + 4 * n, m3::END);
        log!(
            "  TA: {n} words, FIFO {} bytes free",
            cmd::free_bytes(e.regs.read32(STATUS))
        );
        let done = run(e, vram, &w[..12 + n], m3::ta::DONE, None, 200_000)
            .0
            .is_some();
        e.regs.write32(MBX + r85c.0, r85c.1);
        dump(vram, "regions", TA_REGIONS, 6);
        let list = load32_volatile(vram, TA_REGIONS + 4).unwrap_or(0) as usize;
        dump(vram, "first list", list & m3::ADDR_MASK as usize, 8);
        let ptr = load32_volatile(vram, list & m3::ADDR_MASK as usize).unwrap_or(0);
        dump(
            vram,
            "its object",
            PARAMS + 4 * (ptr & 0x001F_FFFF) as usize,
            words,
        );
        if !done {
            log!("  the TA never said done: no 3D render");
            return false;
        }
        let (mut t3, n) = table(level);
        for p in &mut t3[..n] {
            if p.0 == 0x608 {
                p.1 = TA_REGIONS as u32;
            }
        }
        run(e, vram, &t3[..n], RENDER_DONE, Some(probe), 200_000)
            .1
            .is_some()
    }

    /// Dell's start table (level 0), with the likely target register (1),
    /// and with the GLES table's extras too (2); 3 is Dell's with `0x61C`
    /// as the GLES table sets it (`0x00020007`, Dell's `0x00020004`;
    /// `libGLES_CL.dll` `0x0226175c`), 4 that and the extras and the GLES
    /// table's `0x610 = 0x30001` (no scene pointer), `0x64C = 0`. The start
    /// stays last.
    #[inline(never)]
    fn table(level: u8) -> ([(usize, u32); 24], usize) {
        let dell = m3::start_table(REGIONS as u32, PARAMS as u32, W, H, STRIDE as u32);
        let mut t = [(0usize, 0u32); 24];
        let mut n = 0;
        for &p in &dell[..14] {
            t[n] = match p {
                (0x61C, _) if level >= 3 => (0x61C, 0x0002_0007),
                (0x610, _) if level == 4 => (0x610, 0x0003_0001),
                p => p,
            };
            n += 1;
        }
        let more: &[(usize, u32)] = if level == 0 || level == 3 {
            &[]
        } else if level == 4 {
            &[
                (0x65C, TARGET as u32),
                (0x6D0, 1),
                (0x6DC, 1),
                (0x648, 1),
                (0x614, 0x1E3C_E508),
                (0x618, 0x1E3C_E508),
                (0x64C, 0),
            ]
        } else if level == 2 {
            &[
                (0x65C, TARGET as u32),
                (0x6D0, 1),
                (0x6DC, 1),
                (0x648, 1),
                (0x614, 0x1E3C_E508),
                (0x618, 0x1E3C_E508),
            ]
        } else {
            &[(0x65C, TARGET as u32)]
        };
        for &p in more {
            t[n] = p;
            n += 1;
        }
        t[n] = dell[14];
        (t, n + 1)
    }

    /// How far a colour reaches from the top-left along row 2 and column 2.
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
        let clk0 = e.regs.read32(MBXCLK);
        log!("{MARKER}: the MBX 3D core's first triangle (ADR-0024 M4, version {VERSION})");
        log!(
            "core ID 0x{:08x}, MBXCLK 0x{clk0:x}, PLLSTAT 0x{:x}, status 0x{:08x}, enable 0x{:08x}, 0x804 0x{:08x}",
            e.regs.read32(MBX + reg::ID),
            e.regs.read32(PLLSTAT),
            e.regs.read32(STATUS),
            e.regs.read32(ENABLE),
            e.regs.read32(R804)
        );
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();

        // 1-2: the parameter memory, clock as found, then with the 3D clock on.
        let mut ready = false;
        for (name, clk) in [("as found", clk0), ("3D clock on", with_3d(clk0))] {
            rt::check_in();
            if clk != clk0 && e.regs.read32(PLLSTAT) & 4 == 0 {
                // The core PLL feeds the MBX: not locked, not tried.
                log!(
                    "P: PLLSTAT 0x{:x}: the core PLL isn't locked, the 3D clock not tried",
                    e.regs.read32(PLLSTAT)
                );
                break;
            }
            e.regs.write32(MBXCLK, clk);
            rt::delay_us(1_000);
            e.reset(rt::delay_us);
            log!("P: parameter memory set-up, MBXCLK 0x{clk:x} ({name})");
            ready = pm_init(&mut e, vram);
            if ready {
                break;
            }
        }

        // 3: the red screen-covering triangle, try A then B.
        let background = [
            Vertex::flat(0.0, 0.0, 0xFFFF_0000),
            Vertex::flat(2048.0, 0.0, 0xFFFF_0000),
            Vertex::flat(0.0, 2048.0, 0xFFFF_0000),
        ];
        let mut drew = false;
        let mut level = 0;
        if ready {
            for (name, x) in [
                ("A: Dell's table alone", 0),
                ("A': Dell's table + 0x65C", 1),
                ("B: + the GLES extras", 2),
            ] {
                rt::check_in();
                if !clear(&mut e, vram, GREY) {
                    log!("{name}: the 2D clear didn't finish: stopping");
                    break;
                }
                let (t, n) = table(x);
                log!("{name}: red screen-covering triangle, {COLS} x {ROWS} grid of 8 x 16 px");
                drew = draw(&mut e, vram, TSP_DELL, &background, &t[..n], (8, 8));
                if drew {
                    level = x;
                    let (across, down) = reach(vram, GREY);
                    log!(
                        "  drawn {across} px across, {down} down; (8,8) 0x{:08x}, (240,320) 0x{:08x}, (470,630) 0x{:08x}",
                        px(vram, 8, 8),
                        px(vram, 240, 320),
                        px(vram, 470, 630)
                    );
                    log!(
                        "  tile size from the reach: across {:?}, down {:?}",
                        tile_size(across, COLS, W),
                        tile_size(down, ROWS, H)
                    );
                    break;
                }
            }
        } else {
            log!("the parameter memory never said ready: no render tried");
        }

        // 4: red, green and blue corners over grey (shading), if it drew.
        let mut shown = false;
        if drew {
            rt::check_in();
            let _ = clear(&mut e, vram, GREY);
            let tri = [
                Vertex::flat(240.0, 80.0, 0xFFFF_0000),
                Vertex::flat(420.0, 560.0, 0xFF00_FF00),
                Vertex::flat(60.0, 560.0, 0xFF00_00FF),
            ];
            let (t, n) = table(level);
            log!(
                "C: red/green/blue triangle (240,80) (420,560) (60,560), TSP bit 22 (Gouraud) set"
            );
            shown = draw(
                &mut e,
                vram,
                TSP_DELL | TSP_GOURAUD,
                &tri,
                &t[..n],
                (240, 400),
            );
            log!(
                "  near the corners 0x{:08x} 0x{:08x} 0x{:08x}, middle 0x{:08x}, outside 0x{:08x}",
                px(vram, 240, 90),
                px(vram, 405, 552),
                px(vram, 75, 552),
                px(vram, 240, 400),
                px(vram, 20, 20)
            );
        }

        // X: through the tile accelerator (ADR-0024 M5): the GLES clear
        // (version 15 drew it), then a textured triangle.
        if drew {
            rt::check_in();
            let _ = clear(&mut e, vram, GREY);
            log!("X1: the GLES clear through the TA");
            let ok = through_ta(
                &mut e,
                vram,
                level,
                &m3::ta::clear_stream(W, H),
                10,
                (240, 320),
            );
            log!(
                "X1: {}; (240,320) 0x{:08x}",
                if ok { "drawn" } else { "nothing drawn" },
                px(vram, 240, 320)
            );
            for k in 0..256 * 256 / 2 {
                // Two texels a word, texel k holding k.
                let t = 2 * k as u32;
                let _ = store32_volatile(vram, TEXTURE + 4 * k, t | (t + 1) << 16);
            }
            cpu::drain_write_buffer();
            let v = |x: f32, y: f32, u: f32, v: f32| m3::tex::Vertex {
                x,
                y,
                z: 1.0,
                rhw: 1.0,
                argb: 0xFFFF_C080,
                u,
                v,
            };
            let tri = [
                v(40.0, 40.0, 0.0, 0.0),
                v(440.0, 40.0, 1.0, 0.0),
                v(40.0, 600.0, 0.0, 1.0),
            ];
            let texw = [
                m3::tex::FORMAT_GAME,
                m3::tex::address(TEXTURE as u32),
                m3::tex::COMBINE_GAME,
            ];
            for (name, isp) in [("X2", ISP_ALWAYS), ("X3", m3::tex::ISP_GAME)] {
                rt::check_in();
                let _ = clear(&mut e, vram, GREY);
                log!("{name}: a textured triangle through the TA, ISP 0x{isp:08x}");
                let s = m3::ta::textured_stream(m3::tex::TSP_GAME, isp, texw, &tri);
                let ok = through_ta(&mut e, vram, level, &s, 26, (120, 160));
                log!(
                    "{name}: {}; inside (120,160) 0x{:08x} (60,60) 0x{:08x} (200,100) 0x{:08x}, outside (300,500) 0x{:08x}",
                    if ok { "drawn" } else { "nothing drawn" },
                    px(vram, 120, 160),
                    px(vram, 60, 60),
                    px(vram, 200, 100),
                    px(vram, 300, 500)
                );
            }
        }

        let saved = show_target(&mut e.regs);
        let keypad = arm_keypad();
        let verdict = if drew {
            "DREW"
        } else if ready {
            "parameter memory ready, nothing drawn"
        } else {
            "nothing drawn (see the log)"
        };
        log!(
            "{MARKER}: {verdict} - {} shown until a button",
            if shown {
                "the shaded triangle"
            } else {
                "the target"
            }
        );
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: {verdict}");
        }
        // Unattended (the owner asleep, 2026-10-14): 10 s, or a button.
        hold_until_button(keypad, 10_000_000);
        for (off, v) in saved {
            e.regs.write32(off, v);
        }
        e.reset(rt::delay_us);
        e.regs.write32(MBXCLK, clk0);
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

    /// Holds until a button is pressed and released (`limit_us` at most),
    /// checking in with the loader.
    fn hold_until_button(keypad: Option<Keypad<Mmio>>, limit_us: u32) {
        let Some(mut keypad) = keypad else {
            return;
        };
        let t0 = rt::ticks();
        let mut seen = false;
        while us_since(t0) < limit_us {
            rt::check_in();
            seen |= keypad.acknowledge();
            if seen && keypad.pressed() == Pressed::NONE {
                return;
            }
            rt::delay_us(20_000);
        }
        log!("no button: returning");
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

#[cfg(test)]
mod tests {
    use super::{tile_size, with_3d};

    #[test]
    fn tile_size_from_the_red_reach() {
        // 15 x 20 tiles assumed (32 px); 16-pixel tiles draw 240 x 320.
        assert_eq!(tile_size(240, 15, 480), Some(16));
        assert_eq!(tile_size(320, 20, 640), Some(16));
        assert_eq!(tile_size(480, 15, 480), Some(32));
        assert_eq!(tile_size(0, 15, 480), None);
        assert_eq!(tile_size(250, 15, 480), None, "not whole tiles");
    }

    #[test]
    fn the_3d_clock_keeps_the_divider() {
        // WM's 0x5: divider 2, 2D only; Dell's vote: 2D and 3D.
        assert_eq!(with_3d(0x5), 0x6);
        assert_eq!(with_3d(0x9), 0xA);
    }
}
