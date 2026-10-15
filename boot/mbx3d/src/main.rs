//! Test image (ADR-0024 M4): the first triangle drawn by the 2700G's MBX
//! 3D core, under the loader. Version 26 (ADR-0026, the GPU's clock and a
//! 16-bit target, from static reading 2026-10-15): S26a the MBX clock's
//! divider (Windows Mobile's `MBXCLK` `0x5` is divide-by-2: Dell's driver
//! picks the smallest divider keeping the cores at or under 75 MHz) at 2,
//! 3 and 4, a full-screen and a 600-tile render and a full-screen 2D fill
//! timed at each (divide-by-1 is over Dell's cap: compiled out until the
//! owner decides); S26b (T4) a 16-bit target, `0x650` 1 or 3 and `0x660`
//! in pixels or words, colours, extent and time; S26c (last) the PLLs and
//! clock registers read (never read before), for the system clock.
//! T11b-T13b are done (version 25: mipmaps work, PVRTC's blocks twiddled,
//! depth storage parked).
//! Version 25 (ADR-0026, version 24's open
//! questions, from static reading 2026-10-15): T11b depth stored across
//! renders with the GLES driver's full register set (`0x648` and `0x6D0`
//! 1 and its other extras; `n` 16 and 2400); T12b mipmaps with TEX0 bits
//! 12-9 set (the GLES upload puts `0xB` there without levels, the level
//! value with; version 24 left them 0: run last, the riskiest); T13b
//! PVRTC's block order.
//! T10-T13 are done (version 24: interrupt, bilinear, 32-bit, PVRTC).
//! Version 24 (ADR-0026 T10-T13, from static
//! reading 2026-10-15): T10 render done on GPIO85 with `0x130` bit 15 (Dell
//! enables `0x87F5`; T2 had bit 2 alone after a reset), the line quieted
//! first, then which `MINTENABLE` bit routes the MBX core; T11 depth kept
//! across renders (`0x674`, `0x610` store then load); T12 bilinear
//! filtering (TEX1 bit 31) and mipmaps (TEX1 bits 30-29) with two level
//! layouts and a level-of-detail sweep; T13 32-bit texels (TEX0 code 12)
//! and PVRTC 4 bpp (28) and 2 bpp (16) from hand-made blocks. T5-T7 are
//! done (version 23) and gone.
//! Version 23: the GLES driver's own forms,
//! read statically: blending sets TSP bit 13, a texture with alpha takes a
//! combine word (TEX2) with bit 13 clear, and depth compare and write mask
//! sit in the TSP's low bits (Dell's `0x0E80`: always, no write). T5c tries
//! alpha that way, T7b depth with writes on.
//! Version 22: version 21 found the blend
//! factors (additive drew) but vertex alpha acted as 1, the hand-written
//! textured path drawing, clamping by default, and TSP bit 16 hanging the
//! core (T7 void). Now every timed-out render resets the core and sets the
//! memory manager up again; T5b looks for the alpha source (TSP bits, an
//! ARGB4444 texture's alpha), T6b tries TEX0 bits 31-28 for filtering,
//! T7 (depth) runs again.
//! Version 21 (ADR-0026 device tests T5-T7,
//! word layouts guessed from the PowerVR2's): T5 blending (TSP bits 31-26
//! as source/destination factors, bit 20 "use alpha"), T6 texture
//! filtering and clamping on a hand-written textured triangle with Dell's
//! TSP (bits 14-13, 15-18), T7 depth compare (ISP bits 31-29). T1-T3 are
//! done (versions 19-20) and gone.
//! Version 20: T1 passed in version 19 (unlisted
//! tiles untouched, 5.2 us a tile) and runs again; then T2 (does render
//! done, status bit 2, edge GPIO85, the 2700G's interrupt line?) and T3
//! (the target `0x65C` moved inside the picture, stride kept: does a 4 x 4
//! tile block land there, at row, 256-, 32- and 4-byte offsets?).
//! Version 19 (ADR-0026 device test T1): after
//! the proven red triangle and shaded triangle, a blue full-screen object
//! over grey rendered with the region array listing all tiles, one tile,
//! a 10 x 10 block and a 600-tile band: are unlisted tiles left untouched,
//! and how does the render time scale with the tiles listed? The tile
//! accelerator stage (versions 14-18, done) is gone.
//! Version 18: version 17 drew the first
//! textured triangle (through the TA, with the GLES driver's own draw's
//! words; the game's TSP `0x22640200` never draws). Now that triangle with
//! the TA's object logged from the first tile it reached, then TEX0's
//! fields swept (bits 15-12, then 31-28) against the texel ramp, 12
//! pixels logged each, to decode the texture formats. Left on screen: the
//! triangle with Dell's words.
//! Version 17: version 16's textured triangle
//! went through the TA but landed in no tile (every header bit 25, no
//! list); the vertex order is confirmed by the GLES driver's own textured
//! draw (`0x0226eae4`: x, y, z, 1/w, colour, u, v). Now with the state
//! words as that draw sends them: the clip word (full screen) and Dell's
//! texture words (TSP `0x22220E80`), the game's words with the clip, and
//! the game's TSP with Dell's texture words.
//! Version 16: version 15 drew the GLES clear
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

/// `MBXCLK` with the 3D clock on, the divider kept (one definition, in the
/// display driver's `clock` module, ADR-0026 D8).
pub use rp_drv_mbx_display::clock::with_3d;

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
    const VERSION: u32 = 26;
    const MBXCLK: usize = rp_drv_mbx_display::clock::MBXCLK;
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
        regions(vram, |_, _| true);
        run(e, vram, table, RENDER_DONE, Some(probe), 200_000)
            .1
            .is_some()
    }

    /// The region array listing only the tiles `keep` says (x, y), in rows,
    /// each on the list at the parameter buffer's start, the last marked.
    /// Straight into 2700G memory (19 KiB: too big for the stack). Returns
    /// how many it listed.
    #[inline(never)]
    fn regions(vram: &mut [u8], keep: fn(u32, u32) -> bool) -> u32 {
        let total = (0..COLS * ROWS)
            .filter(|k| keep(k % COLS, k / COLS))
            .count() as u32;
        let mut n = 0;
        for k in 0..COLS * ROWS {
            let (x, y) = (k % COLS, k / COLS);
            if keep(x, y) {
                n += 1;
                let entry = m3::region_entry(x, y, PARAMS as u32, n == total);
                store(vram, REGIONS + 8 * (n - 1) as usize, &entry);
            }
        }
        cpu::drain_write_buffer();
        n
    }

    /// A flat square (two triangles) from `(x0, y0)` to `(x1, y1)`.
    #[allow(clippy::too_many_arguments)]
    fn square(
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        argb: u32,
        z: f32,
        isp: u32,
        tsp: u32,
    ) -> [[u32; m3::TRIANGLE_WORDS]; 2] {
        let v = |x, y| Vertex {
            x,
            y,
            z,
            rhw: 1.0,
            argb,
        };
        [
            m3::triangle(isp, tsp, &[v(x0, y0), v(x1, y0), v(x0, y1)]),
            m3::triangle(isp, tsp, &[v(x1, y0), v(x1, y1), v(x0, y1)]),
        ]
    }

    /// The full-screen background triangle in `argb` at depth `z`.
    fn background(argb: u32, z: f32) -> [u32; m3::TRIANGLE_WORDS] {
        background_tsp(argb, z, TSP_DELL)
    }

    /// The background with its own TSP word.
    fn background_tsp(argb: u32, z: f32, tsp: u32) -> [u32; m3::TRIANGLE_WORDS] {
        let v = |x, y| Vertex {
            x,
            y,
            z,
            rhw: 1.0,
            argb,
        };
        m3::triangle(
            ISP_ALWAYS,
            tsp,
            &[v(0.0, 0.0), v(2048.0, 0.0), v(0.0, 2048.0)],
        )
    }

    /// `objs` into the parameter memory (one list, at its start) and the
    /// region array for the tiles `keep` says.
    fn load(vram: &mut [u8], objs: &[(&[u32], u32)], keep: fn(u32, u32) -> bool) {
        let mut at = OBJECT;
        let mut list = [m3::END; 16];
        let n = objs.len().min(15);
        for (k, (w, flags)) in objs[..n].iter().enumerate() {
            store(vram, at, w);
            list[k] = flags | ((at - PARAMS) / 4) as u32;
            at += 4 * w.len();
        }
        store(vram, PARAMS, &list[..=n]);
        regions(vram, keep);
    }

    // Version 26's 2700G memory: a 16-bit picture (480 x 640, not shown).
    const PIC16: usize = 0x00A0_0000;
    const PIC16_STRIDE: usize = 480 * 2;
    /// The core and display PLLs' control registers (system block, Linux
    /// mbxfb's names: `(13 MHz x M) / (N x 2^P)`, M bits 12-7, N 6-4, P 3-1,
    /// enable 0). Windows Mobile's boot code sets them; never read before:
    /// read last.
    const COREPLL: usize = 0x038;
    const DISPPLL: usize = 0x03C;
    const SYSCLKSRC: usize = 0x02C;
    const MEMCLK: usize = 0x04C;
    const M24CLK: usize = 0x050;
    /// `MBXCLK` divide-by-1 runs the MBX cores above the 75 MHz Dell's
    /// driver caps them at (`0x02120724`): off until the owner decides.
    const TRY_DIVIDE_BY_1: bool = false;

    /// `table` with `with` replacing (or, if absent, added before the
    /// start) the pairs named.
    fn patched(
        table: &[(usize, u32)],
        with: &[(usize, u32)],
        buf: &mut [(usize, u32); 40],
    ) -> usize {
        let mut n = 0;
        for &(off, v) in &table[..table.len() - 1] {
            buf[n] = with
                .iter()
                .find(|p| p.0 == off)
                .copied()
                .unwrap_or((off, v));
            n += 1;
        }
        for &p in with {
            if !table.iter().any(|q| q.0 == p.0) {
                buf[n] = p;
                n += 1;
            }
        }
        buf[n] = table[table.len() - 1];
        n + 1
    }

    /// The MBX clock's divider (`MBXCLK` bits 3-2: Windows Mobile's `0x5` is
    /// divide-by-2, 2D only) set to `div`, 3D on; the core reset and the
    /// memory manager set up again (a clock change while running is
    /// untested).
    fn set_divider(e: &mut Engine<Mmio>, vram: &mut [u8], clk0: u32, div: u32) -> u32 {
        let v = (clk0 & !0xF) | (div - 1) << 2 | 2;
        e.regs.write32(MBXCLK, v);
        rt::delay_us(1_000);
        e.reset(rt::delay_us);
        let _ = pm_init(e, vram);
        e.regs.read32(MBXCLK)
    }

    /// S26a: does the 3D (and 2D) core's speed follow `MBXCLK`'s divider?
    /// Windows Mobile runs it at divide-by-2 (Dell's driver: the smallest
    /// divider keeping it at or under 75 MHz); a full-screen render, a
    /// 600-tile band and a full-screen 2D fill timed at divide-by-2, 3, 4.
    #[inline(never)]
    fn s26a(e: &mut Engine<Mmio>, vram: &mut [u8], table: &[(usize, u32)], clk0: u32) {
        let bg = background(0xFFFF_0000, 0.0);
        let p = m3::STRIP_ONE_TRIANGLE;
        let n = if TRY_DIVIDE_BY_1 { 4 } else { 3 };
        for &div in &[2u32, 3, 4, 1][..n] {
            rt::check_in();
            let clk = set_divider(e, vram, clk0, div);
            load(vram, &[(&bg, p)], |_, _| true);
            let full = run(e, vram, table, RENDER_DONE, None, 200_000).0;
            load(vram, &[(&bg, p)], |_, y| (15..25).contains(&y));
            let band = run(e, vram, table, RENDER_DONE, None, 200_000).0;
            let t0 = rt::ticks();
            let filled = clear(e, vram, GREY);
            let fill = us_since(t0);
            log!(
                "  S26a MBXCLK 0x{clk:x} (divide by {div}): 3D 2400 tiles done at {full:?} us, 600 tiles at {band:?} us; 2D full-screen fill {fill} us (finished {filled})"
            );
        }
        let clk = set_divider(e, vram, clk0, 2);
        log!("  S26a back to divide by 2: MBXCLK 0x{clk:x}");
    }

    /// A pixel of the 16-bit picture.
    fn p16(vram: &[u8], x: usize, y: usize) -> u32 {
        let a = PIC16 + y * PIC16_STRIDE + 2 * x;
        let w = load32_volatile(vram, a & !3).unwrap_or(0xDEAD_BEEF);
        if a & 2 == 0 { w & 0xFFFF } else { w >> 16 }
    }

    /// S26b (T4): a 16-bit target. The GLES driver writes `0x650` 6 for a
    /// 32-bit target, 1 or 3 for two other formats; `0x660` is the stride
    /// in pixels or in words. A 4 x 4 tile block (red, green, blue, white
    /// quarters of 32 x 64 px) over a picture filled with `0xAAAA`: the
    /// colours, the drawn extent (are tiles still 8 x 16?), the time.
    #[inline(never)]
    fn s26b(e: &mut Engine<Mmio>, vram: &mut [u8], table: &[(usize, u32)]) {
        let tsp = TSP_DELL;
        let q = [
            square(0.0, 0.0, 16.0, 32.0, 0xFFFF_0000, 0.0, ISP_ALWAYS, tsp),
            square(16.0, 0.0, 32.0, 32.0, 0xFF00_FF00, 0.0, ISP_ALWAYS, tsp),
            square(0.0, 32.0, 16.0, 64.0, 0xFF00_00FF, 0.0, ISP_ALWAYS, tsp),
            square(16.0, 32.0, 32.0, 64.0, 0xFFFF_FFFF, 0.0, ISP_ALWAYS, tsp),
        ];
        let p = m3::STRIP_ONE_TRIANGLE;
        let objs: [(&[u32], u32); 8] = [
            (&q[0][0], p),
            (&q[0][1], p),
            (&q[1][0], p),
            (&q[1][1], p),
            (&q[2][0], p),
            (&q[2][1], p),
            (&q[3][0], p),
            (&q[3][1], p),
        ];
        for (format, stride) in [(1u32, 480u32), (1, 240), (3, 480), (3, 240)] {
            rt::check_in();
            for k in 0..PIC16_STRIDE * 640 / 4 {
                let _ = store32_volatile(vram, PIC16 + 4 * k, 0xAAAA_AAAA);
            }
            cpu::drain_write_buffer();
            let mut buf = [(0usize, 0u32); 40];
            let n = patched(
                table,
                &[(0x650, format), (0x660, stride), (0x65C, PIC16 as u32)],
                &mut buf,
            );
            load(vram, &objs, |x, y| x < 4 && y < 4);
            let t = run(e, vram, &buf[..n], RENDER_DONE, None, 200_000).0;
            let across = (0..128).find(|&x| p16(vram, x, 2) == 0xAAAA);
            let down = (0..128).find(|&y| p16(vram, 2, y) == 0xAAAA);
            log!(
                "  S26b 0x650 {format}, 0x660 {stride}: done at {t:?} us; (2,2) 0x{:04x} (18,2) 0x{:04x} (2,34) 0x{:04x} (18,34) 0x{:04x}; drawn to x {across:?} on row 2, y {down:?} on column 2; row 2 x 30-33 {:04x?}",
                p16(vram, 2, 2),
                p16(vram, 18, 2),
                p16(vram, 2, 34),
                p16(vram, 18, 34),
                [
                    p16(vram, 30, 2),
                    p16(vram, 31, 2),
                    p16(vram, 32, 2),
                    p16(vram, 33, 2)
                ]
            );
        }
    }

    /// S26c (last: never read before): the clock registers, to compute the
    /// system clock the MBX divider divides.
    #[inline(never)]
    fn s26c(e: &mut Engine<Mmio>) {
        let core = e.regs.read32(COREPLL);
        let disp = e.regs.read32(DISPPLL);
        let khz = |v: u32| {
            let (m, n, p) = ((v >> 7) & 63, (v >> 4) & 7, (v >> 1) & 7);
            if n == 0 { 0 } else { 13_000 * m / (n << p) }
        };
        log!(
            "  S26c COREPLL 0x{core:08x} ({} kHz by mbxfb's formula), DISPPLL 0x{disp:08x} ({} kHz), PLLSTAT 0x{:x}, SYSCLKSRC 0x{:x}, MEMCLK 0x{:x}, M24CLK 0x{:x}, MBXCLK 0x{:x}",
            khz(core),
            khz(disp),
            e.regs.read32(PLLSTAT),
            e.regs.read32(SYSCLKSRC),
            e.regs.read32(MEMCLK),
            e.regs.read32(M24CLK),
            e.regs.read32(MBXCLK)
        );
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

        // S26 (ADR-0026, the GPU's clock and a 16-bit target): the MBX
        // divider's effect, T4, then the PLLs read (last: never read).
        if drew {
            let (t, n) = table(level);
            log!("S26a: the MBX clock divider (MBXCLK bits 3-2) against render and fill times");
            s26a(&mut e, vram, &t[..n], clk0);
            log!("S26b (T4): a 16-bit target (0x650 1 or 3, 0x660 in pixels or words)");
            s26b(&mut e, vram, &t[..n]);
            log!("S26c: the PLLs and clock registers");
            s26c(&mut e);
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
