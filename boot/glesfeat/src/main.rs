//! Test image (ADR-0026, static reading of Intel's GLES driver 2026-10-18,
//! docs/hw/2700g.md "Alpha test, fog, the second texture unit"): the MBX
//! 3D core's alpha test, depth "less" without it, and 8-bit grey textures,
//! under the loader. Results only, logged as pixels; no board bit is
//! switched (the 3D clock as `rp-mbx3d` sets it, put back at the end).
//!
//! - **A, alpha test.** The object's second word ("ISP" so far) carries,
//!   by the GLES driver, the alpha test: bits 31-29 the comparison in GL
//!   order, bits 28-21 the reference; TSP bit 14 marks an alpha-tested
//!   object. A red square with vertex alpha 64 over grey, the comparison
//!   always / greater 128 / greater 32 / less 128 / less 0, some also with
//!   TSP bit 14.
//! - **D, depth "less".** Version 23 (T7b) put the depth mode into those
//!   same bits (`mode << 29`, reference 0) and "less" never drew: an alpha
//!   test "alpha < 0" never passes. Repeated with them "always" and the
//!   mode only in TSP bits 11-9, and once the old way (the control).
//! - **L, 8-bit grey textures.** An 8 x 8 texture of one byte value at
//!   format code 10 (the GLES driver's `GL_LUMINANCE`), the combine word
//!   for luminance with modulate (`0x86084610`), white vertices.
//!
//! Every render has a timeout; a timed-out one resets the core and sets
//! the memory manager up again. The target is shown until a button (10 s
//! at most), then everything is put back.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

pub mod frame;

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-GLESFEAT";

/// The second object word with the alpha test the GLES driver forms
/// (`glAlphaFuncx`, `0x02262060`): comparison `func` (GL order, 0 never ..
/// 7 always) in bits 31-29, reference `reference` in bits 28-21.
#[must_use]
pub const fn alpha_word(func: u32, reference: u32) -> u32 {
    (func & 7) << 29 | (reference & 0xFF) << 21
}

/// TSP bit 14: the alpha-tested object kind (the GLES driver sets it when
/// the test can fail).
pub const TSP_ALPHA_TEST: u32 = 1 << 14;

/// Test G (ROADMAP step 3, the give-up audit's item 1): Gouraud at the
/// cores' divide 1 through the tile accelerator against our hand-written
/// objects. Dell's driver draws Gouraud right at 74.75 MHz / 1
/// (`rp-wmprobe`, 2026-10-23) where ours is wrong in every render (GC2);
/// Dell's driver feeds the TA, and the suspect is our list word's layout
/// field (bits 24-21), 4 for every untextured object, Gouraud included.
/// The quad is GC2's ARGB8888 Gouraud kind: the whole 480 x 640 target,
/// corners black, white, blue, yellow (top left, top right, bottom right,
/// bottom left), as `(x, y, colour)`.
pub const QUAD: [(f32, f32, u32); 4] = [
    (0.0, 0.0, 0xFF00_0000),
    (480.0, 0.0, 0xFFFF_FFFF),
    (480.0, 640.0, 0xFF10_20A0),
    (0.0, 640.0, 0xFFFF_E000),
];
/// The quad's two triangles, as the renderer splits it (`rp-mbx3d-render`
/// `encode`): top left, top right, bottom left; top right, bottom right,
/// bottom left.
pub const TRIS: [[usize; 3]; 2] = [[0, 1, 3], [1, 2, 3]];
/// Dell's untextured smooth TSP (`libGLES_CL` `0x02267700`): Dell's TSP
/// with Gouraud's bit 22; bit 8 clear, a colour with every vertex.
pub const TSP_SMOOTH: u32 = 0x2260_0E80;
/// The state block's mask for an untextured draw (the GLES clear's): the
/// TSP, ISP, clip and bit-6 words follow it.
pub const TA_STATE: u32 = 0x1000_0053;
/// The TA's primitive word for a strip (polygon offset 0).
pub const TA_STRIP: u32 = 0x4000_0000;
/// Words in an untextured, coloured TA vertex (the transform stage's 16
/// bytes + 4 for a colour, `0x02272064`): x, y, z, 1/w, colour.
pub const TA_VERTEX_WORDS: usize = 5;
/// The longest [`ta_draw`]: a strip of four vertices.
pub const TA_DRAW_MAX: usize = 6 + 4 * TA_VERTEX_WORDS;
/// The list word's layout field (bits 24-21: the TA writes 0 for the
/// clear's colourless object, 4 flat, 8 one texture unit, 12 two).
pub const LAYOUT: u32 = 0xF << 21;
/// A list word's flags: everything above the object's offset (bits 20-0).
pub const FLAGS: u32 = 0xFFE0_0000;
/// The list word's triangle mask and what sits above the layout field
/// (bits 31-25).
pub const MASK: u32 = 0xFE00_0000;
/// What test G's pictures are filled with before each render: no pixel of
/// the quad has alpha 0x5A, so one left is a pixel the render never wrote.
pub const SENTINEL: u32 = 0x5A5A_5A5A;

/// One TA draw of [`QUAD`]'s `corners` (at most four) as a strip, as the
/// GLES driver sends one (ADR-0024 M5): the state block ([`TA_STATE`],
/// `tsp`, `isp`, `clip`, the bit-6 word 0), the strip word, then x, y, z,
/// 1/w and colour per vertex (z and 1/w 1.0, as `rp-glesfeat` M's draws).
/// Then a `1` goes to the end window. The words and how many.
#[must_use]
pub fn ta_draw(tsp: u32, isp: u32, clip: u32, corners: &[usize]) -> ([u32; TA_DRAW_MAX], usize) {
    let mut s = [0u32; TA_DRAW_MAX];
    s[..6].copy_from_slice(&[TA_STATE, tsp, isp, clip, 0, TA_STRIP]);
    let one = 1.0f32.to_bits();
    let mut n = 6;
    for &k in corners.iter().take(4) {
        let (x, y, c) = QUAD[k];
        s[n..n + TA_VERTEX_WORDS].copy_from_slice(&[x.to_bits(), y.to_bits(), one, one, c]);
        n += TA_VERTEX_WORDS;
    }
    (s, n)
}

/// Way 3's list flags: `ours` with the TA's layout field (bits 24-21), and
/// with every one of the TA's flags (bits 31-21) when its bits 31-25 (the
/// triangle mask) are ours, so a one-triangle object stays one triangle.
/// Our offset (bits 20-0) is kept. The word, and whether all of the TA's
/// flags were taken.
#[must_use]
pub const fn adopt_flags(ours: u32, ta: u32) -> (u32, bool) {
    if ta & MASK == ours & MASK {
        ((ta & FLAGS) | (ours & !FLAGS), true)
    } else {
        ((ours & !LAYOUT) | (ta & LAYOUT), false)
    }
}

/// Test G2 (2026-10-23, after test G): the TA's strip of four was nearly
/// right at divide 1 where our two triangles were wrong in every render;
/// G2 tries each difference alone through our direct path. The list
/// flags the TA wrote for the strip of four (`0x60A098A0`): triangles 0
/// and 1 (mask bits 30 and 29), layout 5.
pub const QUAD_FLAGS: u32 = 0x60A0_0000;
/// Words in the TA's four-corner Gouraud object: ISP, TSP, x/y x4, z x4,
/// 1/w x4, colour x4.
pub const QUAD_WORDS: usize = 22;
/// The strip's corner order (of [`QUAD`]): top left, top right, bottom
/// left, bottom right.
pub const STRIP_ORDER: [usize; 4] = [0, 1, 3, 2];
/// The GLES clear's TSP (no colour per vertex: its colour from the state).
pub const TSP_CLEAR: u32 = 0x2220_6F80;
/// The list flags the TA wrote for the clear (`0x40012E92`): triangle 0,
/// layout 0.
pub const CLEAR_FLAGS: u32 = 0x4000_0000;
/// Words in the TA's clear object: TSP, x/y x3, z x3 (no ISP, no 1/w, no
/// colours).
pub const CLEAR_WORDS: usize = 10;

/// [`QUAD`] as the one object the TA writes for a strip of four, at depth
/// `z` (the TA's: the z it was sent; 1/w 1.0).
#[must_use]
pub fn quad_object(isp: u32, tsp: u32, z: f32) -> [u32; QUAD_WORDS] {
    four_corner_object(isp, tsp, z, STRIP_ORDER.map(|k| QUAD[k]))
}

/// A four-corner object (the TA's form for a strip of four, list flags
/// [`QUAD_FLAGS`]) of `corners` `(x, y, colour)` in strip order, at depth
/// `z`, 1/w 1.0.
#[must_use]
pub fn four_corner_object(
    isp: u32,
    tsp: u32,
    z: f32,
    corners: [(f32, f32, u32); 4],
) -> [u32; QUAD_WORDS] {
    let mut w = [0u32; QUAD_WORDS];
    w[0] = isp;
    w[1] = tsp;
    for (n, &(x, y, c)) in corners.iter().enumerate() {
        w[2 + 2 * n] = x.to_bits();
        w[3 + 2 * n] = y.to_bits();
        w[10 + n] = z.to_bits();
        w[14 + n] = 1.0f32.to_bits();
        w[18 + n] = c;
    }
    w
}

/// Test G3 (2026-10-23, after G2): the colour clear as Dell's GLES driver
/// draws it (static reading, `libGLES_CL.dll` A08): `glClearColorx`
/// (`0x0225E380`) packs the colour as ARGB8888 into context `+0x5F4`;
/// `glClear(GL_COLOR_BUFFER_BIT)` (`0x0225F064`) only marks a clear
/// pending (`+0x6D8` bit 0, the colour copied to `+0x6EC`); the next draw
/// sends it (`0x0226E744`): this state block (TSP, ISP, the bit-6 word: no
/// clip word), then one strip of four over the viewport.
pub const DELL_CLEAR_STATE: u32 = 0x1000_0043;
/// The clear's TSP: Dell's flat opaque TSP with a colour per vertex
/// (`0x22200E00` when a depth clear goes with it; bit 8 set when the colour
/// isn't cleared).
pub const DELL_CLEAR_TSP: u32 = 0x2220_0E80;
/// The clear's corners, in the order Dell's driver sends them: top left,
/// bottom left, top right, bottom right (x from `+0x6DC`/`+0x6E4`, y from
/// `+0x6E0`/`+0x6E8`).
#[must_use]
pub fn dell_clear_corners(width: u32, height: u32, argb: u32) -> [(f32, f32, u32); 4] {
    let (w, h) = (width as f32, height as f32);
    [
        (0.0, 0.0, argb),
        (0.0, h, argb),
        (w, 0.0, argb),
        (w, h, argb),
    ]
}

/// Words in [`dell_clear_draw`].
pub const DELL_CLEAR_WORDS: usize = 5 + 4 * TA_VERTEX_WORDS;

/// The colour clear as Dell's driver streams it to the TA: the state block
/// (`isp`, the bit-6 word 0), the strip word, four 5-word vertices (x, y,
/// the clear depth `z`, 1/w 1.0, the colour). Then a `1` goes to the end
/// window.
#[must_use]
pub fn dell_clear_draw(
    isp: u32,
    width: u32,
    height: u32,
    z: f32,
    argb: u32,
) -> [u32; DELL_CLEAR_WORDS] {
    let mut s = [0u32; DELL_CLEAR_WORDS];
    s[..5].copy_from_slice(&[DELL_CLEAR_STATE, DELL_CLEAR_TSP, isp, 0, TA_STRIP]);
    for (n, (x, y, c)) in dell_clear_corners(width, height, argb)
        .into_iter()
        .enumerate()
    {
        let at = 5 + n * TA_VERTEX_WORDS;
        s[at..at + TA_VERTEX_WORDS].copy_from_slice(&[
            x.to_bits(),
            y.to_bits(),
            z.to_bits(),
            1.0f32.to_bits(),
            c,
        ]);
    }
    s
}

/// Test G3's settings, rising: the core PLL (13 MHz x M / (N x 2^P)) with
/// the cores at divide 1: 74.75 MHz (Dell's `CoreClock` threshold, where
/// Dell's driver draws right), 99.67 (Windows Mobile's system clock) and
/// 133.25 (the aim; the memory right there, GC2). Each with the memory's
/// timing from Dell's rule but its CAS latency [`G_LMTYPE`]'s.
pub const G3_PLLS: [(u32, &str); 3] = [
    (23 << 7 | 4 << 4 | 1, "74.75"),
    (23 << 7 | 3 << 4 | 1, "99.67"),
    (41 << 7 | 4 << 4 | 1, "133.25"),
];

/// The local memory's CAS latency at every divide-1 setting: CL2 (`LMTYPE`
/// `0xAC8`), as GC2 and the soaks run, not Dell's CL3 above 100 MHz: run
/// G1 drew no 3D picture at 108 MHz with CL3 where run 27 drew right at
/// 133 MHz with CL2, and the warm and cold soaks ran 133.25 MHz CL2 with 0
/// BAD. Test G3's first run took Dell's rule and no render finished at
/// 133.25 / 1 (2026-10-23): the test's fault, not the chip's.
pub const G_LMTYPE: u32 = 0xAC8;

/// Test G4 (2026-10-23, after G3): G3 found Dell's own object sequence
/// wrong at divide 1 under our register set-up, where Dell's driver draws
/// it right. Dell's render table at 74.75 MHz / 1 (`rp-wmprobe` run 2,
/// `local/rp-wmprobe-5.txt`, read after Dell's draws), the words that differ
/// from ours (`rp_drv_mbx3d::start_table`), the target's two aside.
pub const DELL_TABLE: [(usize, u32); 7] = [
    (0x610, 0x0003_0001),
    (0x614, 0x1E3C_E500),
    (0x618, 0x1E3C_E500),
    (0x61C, 0x0002_0007),
    (0x648, 1),
    (0x6D8, 0x0900_0000),
    (0x6DC, 0x3F80_0001),
];
/// Dell's target: RGB565 (`0x650` 1) at its stride in words (`0x660`, 480
/// pixels x 2 bytes / 4).
pub const DELL_TARGET16: [(usize, u32); 2] = [(0x650, 1), (0x660, 480 * 2 / 4)];

/// Test G5 (2026-10-23, the owner's method, step 3): the MBX core's words
/// as Dell's driver leaves them after his draws at 74.75 MHz / 1
/// (`rp-wmprobe` v3, run 2, `local/rp-wmprobe-6.txt`; docs/hw/2700g.md
/// "Dell's chip state after his divide-1 draws"), the non-zero ones but
/// the frame's buffer addresses ([`DELL_ADDRESSES`]); every other word of
/// `0x080`, `0x130`, `0x600`-`0x6FC` and `0x800`-`0x87C` read 0. Sorted.
pub const DELL_MBX: [(usize, u32); 20] = [
    (0x130, 0x0000_87F5),
    (0x610, 0x0003_0001),
    (0x614, 0x1E3C_E500),
    (0x618, 0x1E3C_E500),
    (0x61C, 0x0002_0007),
    (0x648, 1),
    (0x650, 1),
    (0x654, 0x01DF_0000),
    (0x658, 0x027F_0000),
    (0x660, 0x0000_00F0),
    (0x6D0, 1),
    (0x6D8, 0x0900_0000),
    (0x6DC, 0x3F80_0001),
    (0x848, 0x00FF_0004),
    (0x84C, 0x01E0_0000),
    (0x850, 0x0280_0000),
    (0x854, 0x3727_C5A0),
    (0x858, 0x3727_C5AC),
    (0x85C, 0x01D4_9CE1),
    (0x860, 0x0020_0030),
];

/// The MBX words that held that frame's buffer addresses under Dell (they
/// differed between his two runs and point into his own memory): never
/// copied, never compared.
pub const DELL_ADDRESSES: [usize; 9] = [
    0x608, 0x60C, 0x65C, 0x824, 0x828, 0x82C, 0x83C, 0x840, 0x844,
];

/// The system block as Dell leaves it at 74.75 MHz / 1 (the same reading),
/// the registers Dell's driver writes (docs/hw/2700g.md "Dell's register
/// writes on the 3D path"): compared with ours at the same clock, logged.
/// Status and read-only words (`PLLSTAT`, `SINTSTAT`, `ODFBSTAT`, `ID`)
/// left out.
pub const DELL_SYS: [(usize, u32); 24] = [
    (0x008, 0x03FD_FFE0),
    (0x014, 0x15),
    (0x018, 0x800F),
    (0x02C, 2),
    (0x030, 1),
    (0x038, 0xBC1),
    (0x03C, 0xEC5),
    (0x048, 1),
    (0x04C, 1),
    (0x050, 0),
    (0x054, 2),
    (0x058, 1),
    (0x05C, 1),
    (0x060, 0x2EFA_0000),
    (0x064, 0),
    (0x06C, 1),
    (0x0C0, 0),
    (0x0C4, 0),
    (0x0C8, 0),
    (0x300, 3),
    (0x304, 3),
    (0x1004, 0x27),
    (0x1014, 0xAC8),
    (0x1024, 0x03FD_FFFC),
];

/// `NINTPW` (`0x014`), the system block's word Dell sets at every clock
/// change: the clock's cycles in 290 ns (`0x0211FB98`: the period in 10 ps
/// units, `10^8 / kHz`, then `29000 / period`, both truncated). 28 at
/// Windows Mobile's 99.67 MHz, 21 at 74.75 (both read under Dell).
#[must_use]
pub const fn nintpw(khz: u32) -> u32 {
    29_000 / (100_000_000 / khz)
}

/// `NINTPW`'s offset.
pub const NINTPW: usize = 0x014;

/// One thing G5 copies from Dell's set-up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DellItem {
    /// An MBX word written before every render (one the start table
    /// writes replaced in it; others written just before it).
    Mbx(usize, u32),
    /// `NINTPW` set by [`nintpw`] after every clock change (Dell's rule),
    /// put back to Windows Mobile's after.
    Nintpw,
    /// The clock change Dell's way: both MBX clocks off (`MBXCLK` 0) just
    /// before the divider is written (`0x02120370`), where ours writes the
    /// new divider over the running clocks.
    ClockOrder,
}

/// G5's items, one bit each in a mask: every MBX word of [`DELL_MBX`] but
/// the target's format and shape (`0x650`-`0x660`: ours draws ARGB8888 at
/// its own stride; Dell's RGB565 target was G4's way 4, wrong), then the
/// system word and the clock order.
pub const DELL_ITEMS: [(&str, DellItem); 18] = [
    ("0x130 enables", DellItem::Mbx(0x130, 0x0000_87F5)),
    ("0x610", DellItem::Mbx(0x610, 0x0003_0001)),
    ("0x614", DellItem::Mbx(0x614, 0x1E3C_E500)),
    ("0x618", DellItem::Mbx(0x618, 0x1E3C_E500)),
    ("0x61c", DellItem::Mbx(0x61C, 0x0002_0007)),
    ("0x648", DellItem::Mbx(0x648, 1)),
    ("0x6d0", DellItem::Mbx(0x6D0, 1)),
    ("0x6d8", DellItem::Mbx(0x6D8, 0x0900_0000)),
    ("0x6dc", DellItem::Mbx(0x6DC, 0x3F80_0001)),
    ("0x848", DellItem::Mbx(0x848, 0x00FF_0004)),
    ("0x84c", DellItem::Mbx(0x84C, 0x01E0_0000)),
    ("0x850", DellItem::Mbx(0x850, 0x0280_0000)),
    ("0x854", DellItem::Mbx(0x854, 0x3727_C5A0)),
    ("0x858", DellItem::Mbx(0x858, 0x3727_C5AC)),
    ("0x85c", DellItem::Mbx(0x85C, 0x01D4_9CE1)),
    ("0x860", DellItem::Mbx(0x860, 0x0020_0030)),
    ("NINTPW by Dell's rule", DellItem::Nintpw),
    ("Dell's clock order", DellItem::ClockOrder),
];

/// Every item.
pub const DELL_ALL: u32 = (1 << DELL_ITEMS.len()) - 1;

/// The most checks [`narrow`] makes (a bound on the run's time: each is a
/// reference and 20 renders at one setting, about a minute).
pub const NARROW_CHECKS: u32 = 30;

/// Where one render's buffers sit in the 2700G's 16 MiB (test G6: the
/// owner's leads after G5, 2026-10-23: placement, the parameter memory's
/// set-up, the render's flow).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    /// The render target.
    pub target: usize,
    /// The target is RGB565 (`0x650` 1, its stride 2 bytes a pixel), not
    /// ARGB8888.
    pub rgb565: bool,
    /// The parameter memory (objects at its start).
    pub pm: usize,
    /// Its size in bytes.
    pub pm_size: u32,
    /// The parameter memory manager's page table (16 KiB cleared).
    pub pm_table: usize,
    /// The region array the 3D stage reads.
    pub regions: usize,
    /// The TA's tail buffer (64 KiB).
    pub tail: usize,
    /// The region array the TA writes.
    pub ta_regions: usize,
}

/// Ours, as every test before G6 (`rp-mbx3d`'s, proven there).
pub const OURS_LAYOUT: Layout = Layout {
    target: 0x0020_0000,
    rgb565: false,
    pm: 0x0060_0000,
    pm_size: 0x0010_0000,
    pm_table: 0x0070_0000,
    regions: 0x00D1_0000,
    tail: 0x00A0_0000,
    ta_regions: 0x00A2_0000,
};

/// Dell's parameter memory as his driver set it at 74.75 MHz / 1
/// (`rp-wmprobe` v3, run 2: `0x83C` `0x22B000` the base, `0x828` `0x22C`
/// the first page, `0x82C` `0xA2A` the last, so 2,048 pages, **8 MiB**;
/// `0x824` `0x1F8000` the page table), the rest ours, the target moved out
/// of its way (to Dell's run-1 target place), and the TA's buffers too
/// (128 KiB aligned; this way draws without the TA).
pub const DELL_PM_LAYOUT: Layout = Layout {
    target: 0x00B6_9000,
    pm: 0x0022_B000,
    pm_size: 0x0080_0000,
    pm_table: 0x001F_8000,
    tail: 0x00CA_0000,
    ta_regions: 0x00CC_0000,
    ..OURS_LAYOUT
};

/// Dell's places, all of them (the same reading): his parameter memory,
/// his RGB565 target at `0x65C` `0xE9D000`, his region array at `0x608` /
/// `0x844` `0xF35000`, his TA tail at `0x840` `0xF40000`.
pub const DELL_LAYOUT: Layout = Layout {
    target: 0x00E9_D000,
    rgb565: true,
    regions: 0x00F3_5000,
    tail: 0x00F4_0000,
    ta_regions: 0x00F3_5000,
    ..DELL_PM_LAYOUT
};

/// Test G6's layouts in way order: ours, Dell's parameter memory, Dell's
/// places (the direct path), Dell's places through the TA (his flow).
pub const G6_LAYOUTS: [Layout; 4] = [OURS_LAYOUT, DELL_PM_LAYOUT, DELL_LAYOUT, DELL_LAYOUT];

/// Test G6 moves the label (scanned out from the first moment, RGB565 480 x
/// 640) out of Dell's 8 MiB parameter memory.
pub const G6_LABEL: usize = 0x0010_0000;
/// And the reference picture (ARGB8888 480 x 640, or RGB565).
pub const G6_REF: usize = 0x00A3_0000;
/// Test G7's reference: clear of Dell's captured frame (his target at
/// `0xB27000`, his parameter memory `0x291000`-`0xA91000`, his lists from
/// `0x564000`: `local/g7-dell-frame.bin`, 2026-10-23) and of ours (region
/// array `0xD10000`); the image checks the frame it carries against it.
pub const G7_REF: usize = 0x00D3_0000;

/// How a render (or the memory manager's set-up) is waited for (test G8,
/// 2026-10-23: G7 found Dell's own frame wrong at divide 1 under our
/// loader, where his driver, which never touches the chip while it
/// renders, draws it right; ours read the status register in a tight loop
/// the whole time).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wait {
    /// The status read again and again, as fast as the bus allows (ours,
    /// every test before G8).
    Tight,
    /// Nothing read until [`QUIET_US`] have passed, then the status once
    /// (Dell's way, near enough: his interrupt thread reads it once).
    Quiet,
    /// The status read once a millisecond.
    EveryMs,
    /// Test G12: the CPU kept off the bus while the chip renders (a count
    /// down in registers and the cache, [`SILENT_SPINS`] of them, not the
    /// OS timer read through the bus as `delay_us` does: Windows Mobile
    /// sleeps meanwhile), then the status once.
    Silent,
}

/// [`Wait::Silent`]'s count: past the slowest render (19.9 ms) at any
/// speed the XScale runs (12 million turns of a decrement loop: about 60 ms
/// at 624 MHz, 175 ms at 208), under the 200 ms every render is allowed.
pub const SILENT_SPINS: u32 = 12_000_000;

/// Test G12: the PXA27x's peripheral clocks the loader leaves on and
/// Windows Mobile has off (G9's `CKEN`, `0x01D91EE3` against `0x00D91AA3`):
/// the FFUART (bit 6), the USB host (bit 10) and bit 24.
pub const LOADER_ONLY_CLOCKS: u32 = 1 << 6 | 1 << 10 | 1 << 24;

/// Whether the image started on the chip as Dell's driver sets it at boot
/// when Windows Mobile has his `CoreClock` (`rp-wmprobe` run 1): the core
/// PLL locked (`PLLSTAT` bit 2), the cores at divide 1 (`MBXCLK` bits 3-2
/// zero) with a clock running (bits 1-0 not zero; `0x0`, every clock off,
/// is the emulator's start, not Dell's chip). Tests G5 and G14.
#[must_use]
pub const fn dells_divide_1(mbxclk: u32, pllstat: u32) -> bool {
    pllstat & 4 != 0 && mbxclk >> 2 & 3 == 0 && mbxclk & 3 != 0
}

/// Test G13's ways: (Dell's frame, not ours; the display at Windows
/// Mobile's pixel clock, `DISPPLL` `0xEC5` (56.4 Hz), not the loader's
/// `0xFC5` (60.3 Hz): G9's last 2700G difference outside the MBX block; the
/// display's fetch shares local memory with the 3D core).
pub const G13_WAYS: [(bool, bool); 4] =
    [(false, false), (true, false), (false, true), (true, true)];

/// Test G12's ways: (Dell's frame, not ours; the CPU silent while the
/// chip renders; [`LOADER_ONLY_CLOCKS`] off during each render).
pub const G12_WAYS: [(bool, bool, bool); 5] = [
    (false, false, false),
    (true, false, false),
    (true, true, false),
    (true, false, true),
    (true, true, true),
];

/// Test G9: the system block's offsets that froze Windows Mobile when read
/// there (`rp-wmgpu` v3-v6, docs/hw/2700g.md: MBX offsets read on the
/// system block), never read by either side's whole-block dump.
pub const SYS_FREEZE: [usize; 4] = [0x12C, 0x30C, 0x85C, 0xF00];

/// Test G9: the system block's words both dumps read: every register it
/// is known to have (Linux mbxfb's names below the display, as
/// `rp-gpubench`'s snapshot and `rp-wmprobe`'s module read them safely),
/// `0x080`-`0x088` (read safely under Windows Mobile), Dell's `0x0C0`-`0x0C8`
/// resets and `0x300`/`0x304`. Not the holes: reading every word stalled
/// the emulator at `0x9C0` and froze Windows Mobile at `0x08C` (2026-10-23).
pub const SYS_WORDS: [usize; 56] = [
    0x000, 0x004, 0x008, 0x00C, 0x014, 0x018, 0x01C, 0x020, 0x024, 0x02C, 0x030, 0x034, 0x038,
    0x03C, 0x040, 0x044, 0x048, 0x04C, 0x050, 0x054, 0x058, 0x05C, 0x060, 0x064, 0x068, 0x06C,
    0x070, 0x074, 0x078, 0x080, 0x084, 0x088, 0x0C0, 0x0C4, 0x0C8, 0x204, 0x210, 0x214, 0x218,
    0x220, 0x224, 0x228, 0x300, 0x304, 0xFF0, 0x1004, 0x1008, 0x100C, 0x1010, 0x1014, 0x1018,
    0x101C, 0x1020, 0x1024, 0x1028, 0x102C,
];

/// Test G10: when the CPU touches the 2700G's memory around a render at
/// the cores' divide 1 (G9, 2026-10-23: no register of the chip's differs
/// from Dell's beyond those G5-G7 copied, yet his frame draws wrong under
/// the loader; every check so far wrote the scene and read the picture
/// through the CPU's bus with the chip at divide 1, and the wrong words are
/// grey, one byte in all three channels).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum G10Mode {
    /// Written, rendered and read at divide 1 (every test before G10).
    AsBefore,
    /// Written at Windows Mobile's clock, the clock changed for the render
    /// alone, back before anything is read.
    RenderOnly,
    /// Written and rendered at divide 1, the clock put back before the
    /// picture is read.
    ReadAtWm,
}

/// Test G10's ways: (Dell's frame, not ours; the mode).
pub const G10_WAYS: [(bool, G10Mode); 4] = [
    (false, G10Mode::AsBefore),
    (false, G10Mode::RenderOnly),
    (true, G10Mode::RenderOnly),
    (false, G10Mode::ReadAtWm),
];

/// Test G11: chip select 2's half of `MSC1` (the 2700G's second interface,
/// Dell's `SRAMChipSelect` 2) as Dell's driver sets it at 74.75 MHz / 1
/// (`rp-wmprobe` v5 run 2), and as Windows Mobile leaves it at its clock
/// (run 1; the loader's too, G9). The loader never changed it at divide 1.
pub const DELL_CS2_DIV1: u32 = 0x7DF1;
/// See [`DELL_CS2_DIV1`].
pub const WM_CS2: u32 = 0x2481;

/// Test G11's ways: (Dell's frame, not ours; Dell's CS2 timing at divide
/// 1; with his NINTPW rule and clock order too).
pub const G11_WAYS: [(bool, bool, bool); 4] = [
    (false, false, false),
    (false, true, false),
    (true, true, false),
    (true, true, true),
];

/// [`Wait::Quiet`]'s wait: past the slowest render seen (Dell's frame,
/// 19.9 ms at 74.75 / 1; G3's quad at 12.6) with room to spare.
pub const QUIET_US: u32 = 60_000;

/// Test G8's ways: (Dell's frame, not ours; how the render is waited for).
pub const G8_WAYS: [(bool, Wait); 5] = [
    (false, Wait::Tight),
    (false, Wait::Quiet),
    (false, Wait::EveryMs),
    (true, Wait::Quiet),
    (true, Wait::EveryMs),
];

/// The MBX words a frame carries that the image never writes directly:
/// the parameter memory manager's (`0x820`-`0x83C`), set up through
/// [`rp_drv_mbx3d::pm_init_table`]'s sequence from the frame's
/// `0x824`/`0x828`/`0x82C`/`0x83C` instead.
pub const PM_BLOCK: core::ops::RangeInclusive<usize> = 0x820..=0x83C;

/// The layout a frame's registers give: its target (`0x65C`, RGB565 when
/// `0x650` is 1), parameter memory (`0x83C`, pages `0x828`-`0x82C`, table
/// `0x824`), region array (`0x608`) and TA buffers (`0x840`, `0x844`; ours
/// if absent). `None` if a register is missing or the pages are reversed.
#[must_use]
pub fn frame_layout(f: &frame::Frame<'_>) -> Option<Layout> {
    let (first, last) = (f.reg(0x828)?, f.reg(0x82C)?);
    if last < first {
        return None;
    }
    Some(Layout {
        target: (f.reg(0x65C)? & 0x01FF_FFFC) as usize,
        rgb565: f.reg(0x650)? == 1,
        pm: f.reg(0x83C)? as usize,
        // `pm_init_table`'s pages: the first at `pm + 4 KiB`, the last
        // `pages - 2` after it.
        pm_size: (last - first + 2) << 12,
        pm_table: f.reg(0x824)? as usize,
        regions: (f.reg(0x608)? & 0x01FF_FFFC) as usize,
        tail: f.reg(0x840).map_or(OURS_LAYOUT.tail, |v| v as usize),
        ta_regions: f.reg(0x844).map_or(OURS_LAYOUT.ta_regions, |v| v as usize),
    })
}

impl Layout {
    /// The target's size in bytes for `w` x `h`.
    #[must_use]
    pub const fn target_bytes(&self, w: u32, h: u32) -> usize {
        (w * h) as usize * if self.rgb565 { 2 } else { 4 }
    }

    /// Every buffer as `(start, bytes, name)` for a `w` x `h` target and a
    /// region array of `tiles` tiles (8 bytes each and the last).
    #[must_use]
    pub const fn spans(&self, w: u32, h: u32, tiles: u32) -> [(usize, usize, &'static str); 6] {
        let regions = 8 * (tiles as usize + 1);
        [
            (self.target, self.target_bytes(w, h), "target"),
            (self.pm, self.pm_size as usize, "parameter memory"),
            (self.pm_table, 0x4000, "page table"),
            (self.regions, regions, "region array"),
            (self.tail, 0x1_0000, "TA tail"),
            (self.ta_regions, regions, "TA region array"),
        ]
    }
}

/// The fewest of `full`'s items that still draw right, by halving (Zeller's
/// delta debugging, complements only): the items split into `n` parts;
/// a part whose removal leaves the picture right is dropped and the split
/// coarsened, else the split refined, until single items stand and none
/// can go. `right(mask)` is one check; at most `limit` are made. Returns
/// the mask left and the checks made; the empty mask is never checked
/// (that is our own set-up, known wrong).
pub fn narrow(full: u32, limit: u32, mut right: impl FnMut(u32) -> bool) -> (u32, u32) {
    let mut s = full;
    let mut n = 2u32;
    let mut checks = 0;
    loop {
        let items: u32 = s.count_ones();
        if items <= 1 || checks >= limit {
            return (s, checks);
        }
        let n_now = n.min(items);
        let size = items.div_ceil(n_now);
        let mut reduced = false;
        for part in 0..n_now {
            // The part: the set bits of `s` from index part*size, size of them.
            let mut c = 0u32;
            let mut seen = 0;
            for bit in 0..32 {
                if s >> bit & 1 == 1 {
                    if seen >= part * size && seen < (part + 1) * size {
                        c |= 1 << bit;
                    }
                    seen += 1;
                }
            }
            // At least two parts of at least one item each: never all of
            // `s`, so `t` is never empty (the host test holds it).
            let t = s & !c;
            if c == 0 {
                continue;
            }
            if checks >= limit {
                return (s, checks);
            }
            checks += 1;
            if right(t) {
                s = t;
                n = if n_now > 2 { n_now - 1 } else { 2 };
                reduced = true;
                break;
            }
        }
        if !reduced {
            if n_now >= items {
                return (s, checks);
            }
            n = (2 * n_now).min(items);
        }
    }
}

/// The GLES clear of a `width` x `height` target as the TA writes its
/// object: one triangle covering the target twice over, z 1.0.
#[must_use]
pub fn clear_object(width: u32, height: u32) -> [u32; CLEAR_WORDS] {
    let (w, h) = (2.0 * width as f32, 2.0 * height as f32);
    let one = 1.0f32.to_bits();
    [
        TSP_CLEAR,
        0,
        0,
        w.to_bits(),
        0,
        0,
        h.to_bits(),
        one,
        one,
        one,
    ]
}

/// Whether triangle `t` touches the rectangle `(x0, y0)`-`(x1, y1)`
/// (edges included): the per-tile lists G2 builds as the TA does, an
/// object listed only in the tiles it touches. No triangle edge has every
/// corner strictly outside it, and the bounding boxes overlap.
#[must_use]
pub fn touches(t: [(f32, f32); 3], (x0, y0, x1, y1): (f32, f32, f32, f32)) -> bool {
    let (lo_x, hi_x) = (
        t[0].0.min(t[1].0).min(t[2].0),
        t[0].0.max(t[1].0).max(t[2].0),
    );
    let (lo_y, hi_y) = (
        t[0].1.min(t[1].1).min(t[2].1),
        t[0].1.max(t[1].1).max(t[2].1),
    );
    if hi_x < x0 || lo_x > x1 || hi_y < y0 || lo_y > y1 {
        return false;
    }
    let cross = |a: (f32, f32), b: (f32, f32), p: (f32, f32)| {
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    };
    let s = cross(t[0], t[1], t[2]).signum();
    let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
    (0..3).all(|e| {
        let (a, b) = (t[e], t[(e + 1) % 3]);
        corners.iter().any(|&p| s * cross(a, b, p) >= 0.0)
    })
}

/// A picture held to its reference, word by word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Words that differ.
    pub wrong: u32,
    /// Of those, words with no channel more than one step off.
    pub one_step: u32,
    /// Of those, words still [`SENTINEL`]: never written.
    pub unwritten: u32,
    /// The largest difference in any one channel (alpha included; a wrap
    /// from 0xFF to 0x00 is 255).
    pub max_step: u32,
    /// The first wrong word: (its index, the reference's, the picture's).
    pub first: Option<(u32, u32, u32)>,
    /// Of the wrong words, those on the picture's outer edge (from
    /// [`Tally::add_px`]).
    pub edge: u32,
    /// The first wrong word off the edge, as `first`.
    pub first_inner: Option<(u32, u32, u32)>,
}

impl Tally {
    /// Word `i`: `want` from the reference, `got` from the picture.
    pub fn add(&mut self, i: u32, want: u32, got: u32) {
        if want == got {
            return;
        }
        self.wrong += 1;
        if got == SENTINEL {
            self.unwritten += 1;
        }
        let step = (0..4)
            .map(|c| ((want >> (8 * c)) & 0xFF).abs_diff((got >> (8 * c)) & 0xFF))
            .max()
            .unwrap_or(0);
        if step <= 1 {
            self.one_step += 1;
        }
        self.max_step = self.max_step.max(step);
        if self.first.is_none() {
            self.first = Some((i, want, got));
        }
    }

    /// Pixel `(x, y)` of a `w` x `h` picture, as [`Tally::add`], its place
    /// counted too (on the outer edge or not).
    pub fn add_px(&mut self, (x, y): (u32, u32), (w, h): (u32, u32), want: u32, got: u32) {
        if want == got {
            return;
        }
        let i = y * w + x;
        self.add(i, want, got);
        if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
            self.edge += 1;
        } else if self.first_inner.is_none() {
            self.first_inner = Some((i, want, got));
        }
    }

    /// `o` added to this one (the first wrong words kept if set).
    pub fn merge(&mut self, o: &Self) {
        self.wrong += o.wrong;
        self.one_step += o.one_step;
        self.unwritten += o.unwritten;
        self.edge += o.edge;
        self.max_step = self.max_step.max(o.max_step);
        if self.first.is_none() {
            self.first = o.first;
        }
        if self.first_inner.is_none() {
            self.first_inner = o.first_inner;
        }
    }
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::clock::{self, Speed, with_3d};
    use rp_drv_mbx_display::font::{self, WIDTH};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, layers::Layer, reg as dreg};
    use rp_drv_mbx3d::{self as m3, ISP_ALWAYS, PM_READY, RENDER_DONE, TSP_DELL, Vertex};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, diag, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{
        DellItem, MARKER, QUAD, SENTINEL, TRIS, TSP_ALPHA_TEST, TSP_SMOOTH, Tally, adopt_flags,
        alpha_word, ta_draw,
    };

    use rp_image::crash;

    rp_image::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    // 2700G local memory, as rp-mbx3d (proven there): target, parameter
    // memory and its manager's block, region array, a texture, the fence.
    const TARGET: usize = super::OURS_LAYOUT.target;
    const W: u32 = 480;
    const H: u32 = 640;
    const STRIDE: usize = 480 * 4;
    const PM: usize = super::OURS_LAYOUT.pm;
    const PM_SIZE: u32 = super::OURS_LAYOUT.pm_size;
    const _: () = assert!(PM + PM_SIZE as usize <= m3::PM_REACH);
    const PARAMS: usize = PM;
    const OBJECT: usize = PARAMS + 64;
    const REGIONS: usize = super::OURS_LAYOUT.regions;
    const TEX: usize = 0x00E0_0000;
    /// Test M's second texture (8 x 8, one byte a texel).
    const TEX_B: usize = TEX + 0x1_0000;
    /// The TA's tail buffer (128 KiB aligned, 4 bytes a tile) and the
    /// region array it writes (as `rp-mbx3d` versions 14-18).
    const TAIL: usize = super::OURS_LAYOUT.tail;
    const TAIL_SIZE: usize = 0x1_0000;
    const TA_REGIONS: usize = super::OURS_LAYOUT.ta_regions;
    const FENCE: usize = 0x00FF_F000;
    const COLS: u32 = W / m3::TILE_W;
    const ROWS: u32 = H / m3::TILE_H;

    const STATUS: usize = MBX + reg::STATUS;
    const ENABLE: usize = MBX + reg::ENABLE;
    const CLEAR: usize = MBX + reg::CLEAR;
    const MBXCLK: usize = rp_drv_mbx_display::clock::MBXCLK;
    const PLLSTAT: usize = 0x040;

    const GREY: u32 = 0xFF20_2020;
    const STRIP: u32 = m3::STRIP_ONE_TRIANGLE;

    /// Test G alone instead of A, D, L and M (`RP_GLESFEAT_G=1`; device
    /// build `RP_GLESFEAT_G=1 cargo xtask codegen-variant rp-glesfeat g`).
    const TEST_G: bool = option_env!("RP_GLESFEAT_G").is_some()
        || TEST_G2
        || TEST_G3
        || TEST_G4
        || TEST_G5
        || TEST_G6
        || TEST_G7
        || TEST_G8
        || TEST_G9
        || TEST_G10
        || TEST_G11
        || TEST_G12
        || TEST_G13
        || TEST_G14;
    /// Test G2 instead of G (`RP_GLESFEAT_G2=1`; device build
    /// `RP_GLESFEAT_G2=1 cargo xtask codegen-variant rp-glesfeat g2`).
    const TEST_G2: bool = option_env!("RP_GLESFEAT_G2").is_some();
    /// Test G3 instead of G (`RP_GLESFEAT_G3=1`; device build
    /// `RP_GLESFEAT_G3=1 cargo xtask codegen-variant rp-glesfeat g3`).
    const TEST_G3: bool = option_env!("RP_GLESFEAT_G3").is_some();
    /// Test G4 instead of G (`RP_GLESFEAT_G4=1`; device build
    /// `RP_GLESFEAT_G4=1 cargo xtask codegen-variant rp-glesfeat g4`).
    const TEST_G4: bool = option_env!("RP_GLESFEAT_G4").is_some();
    /// Test G5 instead of G (`RP_GLESFEAT_G5=1`; device build
    /// `RP_GLESFEAT_G5=1 cargo xtask codegen-variant rp-glesfeat g5`).
    const TEST_G5: bool = option_env!("RP_GLESFEAT_G5").is_some();
    /// Test G6 instead of G (`RP_GLESFEAT_G6=1`; device build
    /// `RP_GLESFEAT_G6=1 cargo xtask codegen-variant rp-glesfeat g6`).
    const TEST_G6: bool = option_env!("RP_GLESFEAT_G6").is_some();
    /// Test G7 instead of G (`RP_GLESFEAT_G7=1` and
    /// `RP_GLESFEAT_G7_FRAME=<frame file>`; device build
    /// `cargo xtask g7-frame`, then `RP_GLESFEAT_G7=1
    /// RP_GLESFEAT_G7_FRAME=local/g7-dell-frame.bin cargo xtask
    /// codegen-variant rp-glesfeat g7`).
    const TEST_G7: bool = option_env!("RP_GLESFEAT_G7").is_some();
    /// Test G8 instead of G (`RP_GLESFEAT_G8=1` and, as G7,
    /// `RP_GLESFEAT_G7_FRAME`; device build `RP_GLESFEAT_G8=1
    /// RP_GLESFEAT_G7_FRAME=local/g7-dell-frame.bin cargo xtask
    /// codegen-variant rp-glesfeat g8`).
    const TEST_G8: bool = option_env!("RP_GLESFEAT_G8").is_some();
    /// Test G9 instead of G (`RP_GLESFEAT_G9=1`; device build
    /// `RP_GLESFEAT_G9=1 cargo xtask codegen-variant rp-glesfeat g9`).
    const TEST_G9: bool = option_env!("RP_GLESFEAT_G9").is_some();
    /// Test G10 instead of G (`RP_GLESFEAT_G10=1` and, as G7,
    /// `RP_GLESFEAT_G7_FRAME`; device build `RP_GLESFEAT_G10=1
    /// RP_GLESFEAT_G7_FRAME=local/g7-dell-frame.bin cargo xtask
    /// codegen-variant rp-glesfeat g10`).
    const TEST_G10: bool = option_env!("RP_GLESFEAT_G10").is_some();
    /// Test G11 instead of G (`RP_GLESFEAT_G11=1` and, as G7,
    /// `RP_GLESFEAT_G7_FRAME`; device build `RP_GLESFEAT_G11=1
    /// RP_GLESFEAT_G7_FRAME=local/g7-dell-frame.bin cargo xtask
    /// codegen-variant rp-glesfeat g11`).
    const TEST_G11: bool = option_env!("RP_GLESFEAT_G11").is_some();
    /// Test G12 instead of G (`RP_GLESFEAT_G12=1` and, as G7,
    /// `RP_GLESFEAT_G7_FRAME`; device build `RP_GLESFEAT_G12=1
    /// RP_GLESFEAT_G7_FRAME=local/g7-dell-frame.bin cargo xtask
    /// codegen-variant rp-glesfeat g12`).
    const TEST_G12: bool = option_env!("RP_GLESFEAT_G12").is_some();
    /// Test G13 instead of G (`RP_GLESFEAT_G13=1` and, as G7,
    /// `RP_GLESFEAT_G7_FRAME`; device build `RP_GLESFEAT_G13=1
    /// RP_GLESFEAT_G7_FRAME=local/g7-dell-frame.bin cargo xtask
    /// codegen-variant rp-glesfeat g13`).
    const TEST_G13: bool = option_env!("RP_GLESFEAT_G13").is_some();
    /// Test G14 instead of G (`RP_GLESFEAT_G14=1` and, as G7,
    /// `RP_GLESFEAT_G7_FRAME`; device build `RP_GLESFEAT_G14=1
    /// RP_GLESFEAT_G7_FRAME=local/g7-dell-frame.bin cargo xtask
    /// codegen-variant rp-glesfeat g14`).
    const TEST_G14: bool = option_env!("RP_GLESFEAT_G14").is_some();
    /// Test G14's first way run whatever clock the image starts at
    /// (`RP_GLESFEAT_G14=any-start`): the emulator starts with every 2700G
    /// clock off, never at Dell's divide 1, so its check builds this to
    /// step through the way. Never in a device build: there the way runs
    /// only on the chip as Dell's driver left it.
    const G14_ANY_START: bool = match option_env!("RP_GLESFEAT_G14") {
        Some(v) => matches!(v.as_bytes(), b"any-start"),
        None => false,
    };
    const _: () = assert!(
        !TEST_G14 || cfg!(rp_g7),
        "test G14 needs RP_GLESFEAT_G7_FRAME (cargo xtask g7-frame)"
    );
    const _: () = assert!(
        !TEST_G13 || cfg!(rp_g7),
        "test G13 needs RP_GLESFEAT_G7_FRAME (cargo xtask g7-frame)"
    );
    const _: () = assert!(
        !TEST_G12 || cfg!(rp_g7),
        "test G12 needs RP_GLESFEAT_G7_FRAME (cargo xtask g7-frame)"
    );
    const _: () = assert!(
        !TEST_G11 || cfg!(rp_g7),
        "test G11 needs RP_GLESFEAT_G7_FRAME (cargo xtask g7-frame)"
    );
    const _: () = assert!(
        !TEST_G10 || cfg!(rp_g7),
        "test G10 needs RP_GLESFEAT_G7_FRAME (cargo xtask g7-frame)"
    );
    const _: () = assert!(
        !TEST_G8 || cfg!(rp_g7),
        "test G8 needs RP_GLESFEAT_G7_FRAME (cargo xtask g7-frame)"
    );
    #[cfg(rp_g7)]
    static FRAME_FILE: &[u8] = include_bytes!(env!("RP_GLESFEAT_G7_FRAME"));
    #[cfg(not(rp_g7))]
    static FRAME_FILE: &[u8] = &[];
    const _: () = assert!(
        !TEST_G7 || cfg!(rp_g7),
        "test G7 needs RP_GLESFEAT_G7_FRAME (cargo xtask g7-frame)"
    );
    /// The emulator's check only (`RP_GLESFEAT_G5_CULPRIT=<item>`): the
    /// fork draws right at every clock, so with this a G5 check counts as
    /// right only when its mask holds that item, and the halving must end
    /// on it (that the search runs and reports what it found). Never in a
    /// device build.
    const G5_CULPRIT: Option<&str> = option_env!("RP_GLESFEAT_G5_CULPRIT");
    /// The label's title and the log's way prefix.
    const G_TITLE: &str = if TEST_G14 {
        "RP-GLESFEAT TEST G14"
    } else if TEST_G13 {
        "RP-GLESFEAT TEST G13"
    } else if TEST_G12 {
        "RP-GLESFEAT TEST G12"
    } else if TEST_G11 {
        "RP-GLESFEAT TEST G11"
    } else if TEST_G10 {
        "RP-GLESFEAT TEST G10"
    } else if TEST_G9 {
        "RP-GLESFEAT TEST G9"
    } else if TEST_G8 {
        "RP-GLESFEAT TEST G8"
    } else if TEST_G7 {
        "RP-GLESFEAT TEST G7"
    } else if TEST_G6 {
        "RP-GLESFEAT TEST G6"
    } else if TEST_G5 {
        "RP-GLESFEAT TEST G5"
    } else if TEST_G4 {
        "RP-GLESFEAT TEST G4"
    } else if TEST_G3 {
        "RP-GLESFEAT TEST G3"
    } else if TEST_G2 {
        "RP-GLESFEAT TEST G2"
    } else {
        "RP-GLESFEAT TEST G"
    };
    const G_NAME: &str = if TEST_G14 {
        "G14"
    } else if TEST_G13 {
        "G13"
    } else if TEST_G12 {
        "G12"
    } else if TEST_G11 {
        "G11"
    } else if TEST_G10 {
        "G10"
    } else if TEST_G9 {
        "G9"
    } else if TEST_G8 {
        "G8"
    } else if TEST_G7 {
        "G7"
    } else if TEST_G6 {
        "G6"
    } else if TEST_G5 {
        "G5"
    } else if TEST_G4 {
        "G4"
    } else if TEST_G3 {
        "G3"
    } else if TEST_G2 {
        "G2"
    } else {
        "G"
    };
    /// Draws per way at divide 1 (GC2's rule 6).
    const G_DRAWS: u32 = 20;
    /// Test G's reference picture (ARGB8888, as the target).
    const REF: usize = if TEST_G7
        || TEST_G8
        || TEST_G10
        || TEST_G11
        || TEST_G12
        || TEST_G13
        || TEST_G14
    {
        super::G7_REF
    } else if TEST_G6 {
        super::G6_REF
    } else {
        0x0080_0000
    };
    const _: () = assert!(
        TEST_G6
            || TEST_G7
            || TEST_G8
            || TEST_G10
            || TEST_G11
            || TEST_G12
            || TEST_G13
            || TEST_G14
            || REF + STRIDE * H as usize <= TAIL
    );
    /// The label scanned out from the first moment: RGB565 (a 32-bit
    /// screen corrupts 32-bit renders: ADR-0023), after the target.
    const LABEL: usize =
        if TEST_G6
            || TEST_G7
            || TEST_G8
            || TEST_G10
            || TEST_G11
            || TEST_G12
            || TEST_G13
            || TEST_G14
        {
            super::G6_LABEL
        } else {
            0x0040_0000
        };
    const LABEL_STRIDE: usize = 480 * 2;
    const _: () = assert!(
        TEST_G6
            || TEST_G7
            || TEST_G8
            || TEST_G10
            || TEST_G11
            || TEST_G12
            || TEST_G13
            || TEST_G14
            || TARGET + STRIDE * H as usize <= LABEL
    );
    const _: () = assert!(
        TEST_G6
            || TEST_G7
            || TEST_G8
            || TEST_G10
            || TEST_G11
            || TEST_G12
            || TEST_G13
            || TEST_G14
            || LABEL + LABEL_STRIDE * H as usize <= PM
    );
    /// The label's text: lines, the font's scale, colours (RGB565).
    const LABEL_LINES: usize = 16;
    const LABEL_SCALE: usize = 3;
    const LABEL_LINE_H: usize = 32;
    const LABEL_TOP: usize = 24;
    const INK: u32 = 0xFFFF;
    const PAPER: u32 = 0x0010;
    /// CS3 timing (RDF, RDN, RRR): Windows Mobile's, and Dell's tier for
    /// 25-99 MHz (`rp-gpubench`'s `CS3_WM` and `CS3_LONG`).
    const CS3_WM: (u32, u32, u32) = (8, 4, 2);
    const CS3_LONG: (u32, u32, u32) = (13, 9, 7);

    /// The layout the current way draws in (an index into
    /// [`super::G6_LAYOUTS`]; 0, ours, for every test but G6).
    static LAY: rt::Global<super::Layout> = rt::Global::new(super::OURS_LAYOUT);

    fn lay() -> super::Layout {
        LAY.with(|l| *l)
    }

    fn set_lay(l: super::Layout) {
        LAY.with(|v| *v = l);
    }

    /// How [`run`] waits (test G8; [`super::Wait::Tight`] otherwise).
    static WAIT: rt::Global<super::Wait> = rt::Global::new(super::Wait::Tight);

    /// Test G10: the divide-1 setting a per-render way changes to (its core
    /// PLL; 0 when none), and the clock changes made and those after which
    /// the display wasn't sending.
    static G10_PLL: rt::Global<(u32, u32, u32)> = rt::Global::new((0, 0, 0));

    /// Test G12: the current way turns the loader-only clocks off during
    /// each render.
    static G12_CLOCKS: rt::Global<bool> = rt::Global::new(false);

    /// Test G11: the current way changes CS2's timing to Dell's at divide 1.
    static G11_CS2: rt::Global<bool> = rt::Global::new(false);

    /// Ways a test ran before [`test_settings`]'s (G14's first, at Dell's
    /// own clock): its ways and steps numbered after them.
    static WAYS_BEFORE: rt::Global<u32> = rt::Global::new(0);

    /// G14's first way for the summary: not run (`Some(None)`), or the
    /// renders right of [`G_DRAWS`]; `None` in every other test.
    static FIRST_WAY: rt::Global<Option<Option<u32>>> = rt::Global::new(None);

    /// Test G7's frame, if the image carries one that parses.
    fn frame() -> Option<super::frame::Frame<'static>> {
        super::frame::parse(FRAME_FILE).ok()
    }

    /// The target's words (two RGB565 pixels a word).
    fn target_words() -> usize {
        lay().target_bytes(W, H) / 4
    }

    /// Pixels a target word holds.
    fn per_word() -> u32 {
        if lay().rgb565 { 2 } else { 1 }
    }

    fn store(vram: &mut [u8], at: usize, words: &[u32]) {
        for (k, &w) in words.iter().enumerate() {
            let _ = store32_volatile(vram, at + 4 * k, w);
        }
    }

    fn px(vram: &[u8], x: u32, y: u32) -> u32 {
        load32_volatile(vram, lay().target + y as usize * STRIDE + 4 * x as usize)
            .unwrap_or(0xDEAD_BEEF)
    }

    /// Writes `pairs` with `bit` enabled; true if the bit set in time.
    fn run(e: &mut Engine<Mmio>, pairs: &[(usize, u32)], bit: u32, limit_us: u32) -> bool {
        let en = e.regs.read32(ENABLE);
        e.regs.write32(CLEAR, bit);
        e.regs.write32(ENABLE, en | bit);
        // G12: the loader-only clocks off while the chip works.
        let clocks = G12_CLOCKS.with(|c| *c).then(|| {
            let was = rt::clock_enables();
            rt::set_clock_enables(was & !super::LOADER_ONLY_CLOCKS);
            was
        });
        let t0 = rt::ticks();
        for &(off, v) in pairs {
            e.regs.write32(MBX + off, v);
        }
        let mut set = false;
        match WAIT.with(|w| *w) {
            super::Wait::Tight => {
                while us_since(t0) < limit_us {
                    if e.regs.read32(STATUS) & bit != 0 {
                        set = true;
                        break;
                    }
                }
            }
            // G8: the chip left alone while it works, then read once.
            super::Wait::Quiet => {
                rt::delay_us(super::QUIET_US.min(limit_us));
                set = e.regs.read32(STATUS) & bit != 0;
            }
            super::Wait::Silent => {
                let mut n = super::SILENT_SPINS;
                while n != 0 {
                    n = core::hint::black_box(n) - 1;
                }
                set = e.regs.read32(STATUS) & bit != 0;
            }
            super::Wait::EveryMs => {
                while us_since(t0) < limit_us {
                    if e.regs.read32(STATUS) & bit != 0 {
                        set = true;
                        break;
                    }
                    rt::delay_us(1_000);
                }
            }
        }
        if let Some(was) = clocks {
            rt::set_clock_enables(was);
        }
        e.regs.write32(CLEAR, bit);
        e.regs.write32(ENABLE, en);
        set
    }

    fn pm_init(e: &mut Engine<Mmio>, vram: &mut [u8]) -> bool {
        let l = lay();
        for k in 0..4096 {
            let _ = store32_volatile(vram, l.pm_table + 4 * k, 0);
        }
        cpu::drain_write_buffer();
        let t = m3::pm_init_table(l.pm as u32, l.pm_size, l.pm_table as u32);
        run(e, &t, PM_READY, 100_000)
    }

    /// Every tile listing the one object list at the parameter buffer.
    #[inline(never)]
    fn regions(vram: &mut [u8]) {
        let total = COLS * ROWS;
        for k in 0..total {
            let l = lay();
            let entry = m3::region_entry(k % COLS, k / COLS, l.pm as u32, k + 1 == total);
            store(vram, l.regions + 8 * k as usize, &entry);
        }
        cpu::drain_write_buffer();
    }

    /// Dell's start table with the target register (rp-mbx3d's level 1).
    fn table() -> ([(usize, u32); 16], usize) {
        let l = lay();
        let stride = if l.rgb565 { W * 2 } else { STRIDE as u32 };
        let mut dell = m3::start_table(l.regions as u32, l.pm as u32, W, H, stride);
        if l.rgb565 {
            for p in &mut dell {
                if p.0 == 0x650 {
                    p.1 = 1;
                }
            }
        }
        let mut t = [(0usize, 0u32); 16];
        t[..14].copy_from_slice(&dell[..14]);
        t[14] = (0x65C, l.target as u32);
        t[15] = dell[14];
        (t, 16)
    }

    /// `objs` (words, list pointer flags) in order, rendered; true if it
    /// finished. A timeout resets the core (version 21's lesson).
    #[inline(never)]
    fn scene(e: &mut Engine<Mmio>, vram: &mut [u8], objs: &[(&[u32], u32)]) -> bool {
        scene_with(e, vram, objs, &[])
    }

    /// [`scene`] with the start table's words `over` written instead of
    /// ours (test G4: Dell's render table). Each must be a register the
    /// table writes already; one that isn't is logged and left out.
    #[inline(never)]
    fn scene_with(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        objs: &[(&[u32], u32)],
        over: &[(usize, u32)],
    ) -> bool {
        let params = lay().pm;
        let mut at = params + 64;
        let mut list = [m3::END; 16];
        let n = objs.len().min(15);
        for (k, (w, flags)) in objs[..n].iter().enumerate() {
            store(vram, at, w);
            list[k] = flags | ((at - params) / 4) as u32;
            at += 4 * w.len();
        }
        store(vram, params, &list[..=n]);
        regions(vram);
        let (mut t, len) = table();
        for &(off, v) in over {
            // The last pair starts the render: never replaced.
            match t[..len - 1].iter_mut().find(|p| p.0 == off) {
                Some(p) => p.1 = v,
                None => log!("  (0x{off:x} isn't in the start table: not written)"),
            }
        }
        let done = run(e, &t[..len], RENDER_DONE, 200_000);
        if !done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("  (timed out: core reset, memory manager ready again: {ok})");
        }
        done
    }

    fn background(z: f32, tsp: u32) -> [u32; m3::TRIANGLE_WORDS] {
        let v = |x, y| Vertex {
            x,
            y,
            z,
            rhw: 1.0,
            argb: GREY,
        };
        m3::triangle(
            ISP_ALWAYS,
            tsp,
            &[v(0.0, 0.0), v(2048.0, 0.0), v(0.0, 2048.0)],
        )
    }

    /// A flat square (two triangles), `(x0, y0)` to `(x1, y1)`.
    #[allow(clippy::too_many_arguments)]
    fn square(
        (x0, y0, x1, y1): (f32, f32, f32, f32),
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

    /// A: the alpha test on a red square with vertex alpha 64.
    #[inline(never)]
    fn alpha(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let bg = background(0.0, TSP_DELL);
        let cases: [(&str, u32, u32); 7] = [
            ("always, ref 0 (control)", alpha_word(7, 0), TSP_DELL),
            ("greater 128", alpha_word(4, 128), TSP_DELL),
            (
                "greater 128, TSP bit 14",
                alpha_word(4, 128),
                TSP_DELL | TSP_ALPHA_TEST,
            ),
            ("greater 32", alpha_word(4, 32), TSP_DELL),
            ("less 128", alpha_word(1, 128), TSP_DELL),
            (
                "less 128, TSP bit 14",
                alpha_word(1, 128),
                TSP_DELL | TSP_ALPHA_TEST,
            ),
            (
                "less 0 (version 23's depth 'less')",
                alpha_word(1, 0),
                TSP_DELL,
            ),
        ];
        for (name, isp, tsp) in cases {
            rt::check_in();
            let sq = square((100.0, 100.0, 300.0, 300.0), 0x40FF_0000, 0.0, isp, tsp);
            let done = scene(e, vram, &[(&bg, STRIP), (&sq[0], STRIP), (&sq[1], STRIP)]);
            log!(
                "  A {name}: ISP 0x{isp:08x} TSP 0x{tsp:08x}: done {done}; square (200,200) 0x{:08x}, outside (20,20) 0x{:08x}",
                px(vram, 200, 200),
                px(vram, 20, 20)
            );
        }
    }

    /// D: version 23's depth test with the alpha bits "always".
    #[inline(never)]
    fn depth(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let tsp = |mode: u32| (TSP_DELL & !0x0F80) | mode << 9;
        let bg = background(0.1, tsp(7));
        let red = square(
            (100.0, 100.0, 300.0, 300.0),
            0xFFFF_0000,
            0.5,
            ISP_ALWAYS,
            tsp(7),
        );
        for (zb, tenths) in [(0.3f32, 3u32), (0.8, 8)] {
            for (mode, isp, form) in [
                (1u32, ISP_ALWAYS, "alpha always"),
                (3, ISP_ALWAYS, "alpha always"),
                (4, ISP_ALWAYS, "alpha always"),
                (1, 1 << 29, "version 23's form"),
            ] {
                rt::check_in();
                let blue = square(
                    (200.0, 200.0, 400.0, 400.0),
                    0xFF00_00FF,
                    zb,
                    isp,
                    tsp(mode),
                );
                let done = scene(
                    e,
                    vram,
                    &[
                        (&bg, STRIP),
                        (&red[0], STRIP),
                        (&red[1], STRIP),
                        (&blue[0], STRIP),
                        (&blue[1], STRIP),
                    ],
                );
                log!(
                    "  D blue z 0.{tenths} mode {mode} ({form}), ISP 0x{isp:08x} TSP 0x{:08x}: done {done}; overlap (250,250) 0x{:08x}, blue alone (350,350) 0x{:08x}, red alone (150,150) 0x{:08x}",
                    tsp(mode),
                    px(vram, 250, 250),
                    px(vram, 350, 350),
                    px(vram, 150, 150)
                );
            }
        }
    }

    /// L: an 8 x 8 texture of one byte value at format code 10.
    #[inline(never)]
    fn luminance(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let bg = background(0.0, TSP_DELL);
        let v = |x, y, u, v| m3::tex::Vertex {
            x,
            y,
            z: 0.0,
            rhw: 1.0,
            argb: 0xFFFF_FFFF,
            u,
            v,
        };
        for (code, byte, tex2) in [
            (10u32, 0x60u32, 0x8608_4610u32),
            (10, 0xC0, 0x8608_4610),
            (10, 0x60, 0x8608_6610),
        ] {
            rt::check_in();
            for k in 0..16 {
                let _ = store32_volatile(vram, TEX + 4 * k, byte * 0x0101_0101);
            }
            cpu::drain_write_buffer();
            let mut texw = m3::ta::dell_texture(TEX as u32);
            texw[0] = m3::tex::tex0(0, code);
            texw[2] = tex2;
            let tsp = m3::ta::TSP_DELL_TEXTURED;
            let a = m3::tex::triangle(
                ISP_ALWAYS,
                texw,
                tsp,
                &[
                    v(100.0, 100.0, 0.0, 0.0),
                    v(300.0, 100.0, 1.0, 0.0),
                    v(100.0, 300.0, 0.0, 1.0),
                ],
            );
            let b = m3::tex::triangle(
                ISP_ALWAYS,
                texw,
                tsp,
                &[
                    v(300.0, 100.0, 1.0, 0.0),
                    v(300.0, 300.0, 1.0, 1.0),
                    v(100.0, 300.0, 0.0, 1.0),
                ],
            );
            let t = m3::tex::STRIP_GAME;
            let done = scene(e, vram, &[(&bg, STRIP), (&a, t), (&b, t)]);
            log!(
                "  L code {code}, texels 0x{byte:02x}, TEX2 0x{tex2:08x}: done {done}; square (200,200) 0x{:08x}, (120,280) 0x{:08x}",
                px(vram, 200, 200),
                px(vram, 120, 280)
            );
        }
    }

    /// The longest stream [`through_ta`] sends.
    const TA_WORDS: usize = 96;
    /// The most draws in one [`through_ta`].
    const TA_DRAWS: usize = 4;
    /// The most list words [`through_ta`] reads from the first tile.
    const TA_LIST: usize = 6;

    /// What [`through_ta`] did.
    struct TaOut {
        /// The TA said done.
        ta: bool,
        /// The render after it finished.
        done: bool,
        /// The first tile's list words up to its end word ([`m3::END`]
        /// when fewer).
        list: [u32; TA_LIST],
    }

    /// `draws` through the tile accelerator (`rp-mbx3d` version 18's
    /// path: its buffers zeroed, its start table, each draw into its
    /// window followed by a `1` into the end window (each window's offset
    /// counting on, as Dell's driver counts), the scene's end), then, if
    /// the TA said done, the 3D stage on the TA's region array. With
    /// `words`, logs the first tile's list and each object the TA wrote
    /// (`words` of each).
    #[inline(never)]
    fn through_ta(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        draws: &[&[u32]],
        words: usize,
        over: &[(usize, u32)],
    ) -> TaOut {
        let l = lay();
        let mut out = TaOut {
            ta: false,
            done: false,
            list: [m3::END; TA_LIST],
        };
        for k in 0..TAIL_SIZE / 4 {
            let _ = store32_volatile(vram, l.tail + 4 * k, 0);
        }
        for k in 0..2 * (COLS * ROWS + 1) as usize {
            let _ = store32_volatile(vram, l.ta_regions + 4 * k, 0);
        }
        cpu::drain_write_buffer();
        let t = m3::ta::start_table(l.tail as u32, l.ta_regions as u32, W, H, COLS, ROWS);
        let mut w = [(0usize, 0u32); 9 + TA_WORDS + TA_DRAWS + 2];
        w[..9].copy_from_slice(&t);
        let (mut at, mut n, mut ends) = (9, 0, 0);
        for d in draws.iter().take(TA_DRAWS) {
            if n + d.len() > TA_WORDS {
                log!("    the draws exceed {TA_WORDS} words: the rest not sent");
                break;
            }
            for &v in d.iter() {
                w[at] = (m3::ta::PORT - MBX + 4 * n, v);
                at += 1;
                n += 1;
            }
            w[at] = (m3::ta::END_PORT - MBX + 4 * ends, 1);
            at += 1;
            ends += 1;
        }
        let r85c = t[7];
        w[at] = (r85c.0, r85c.1 | m3::ta::SCENE_END);
        w[at + 1] = (m3::ta::PORT - MBX + 4 * n, m3::END);
        out.ta = run(e, &w[..at + 2], m3::ta::DONE, 200_000);
        e.regs.write32(MBX + r85c.0, r85c.1);
        if !out.ta {
            log!("    the TA never said done: no render");
            return out;
        }
        // The first tile the TA put something in, its list, its objects.
        let tile = (0..(COLS * ROWS) as usize)
            .map(|k| l.ta_regions + 8 * k)
            .find(|&a| load32_volatile(vram, a + 4).unwrap_or(0) != 0)
            .unwrap_or(l.ta_regions);
        let list = load32_volatile(vram, tile + 4).unwrap_or(0) as usize & m3::ADDR_MASK as usize;
        for (k, slot) in out.list.iter_mut().enumerate() {
            let ptr = load32_volatile(vram, list + 4 * k).unwrap_or(m3::END);
            *slot = ptr;
            if ptr == m3::END {
                break;
            }
            if words == 0 {
                continue;
            }
            let obj = l.pm + 4 * (ptr & 0x001F_FFFF) as usize;
            let _ = write!(
                RunLog,
                "    TA list word 0x{ptr:08x} (layout {}), object at 0x{obj:x}:",
                (ptr & super::LAYOUT) >> 21
            );
            for j in 0..words {
                let _ = write!(
                    RunLog,
                    " {:08x}",
                    load32_volatile(vram, obj + 4 * j).unwrap_or(0)
                );
            }
            log!("");
        }
        let (mut t3, len) = table();
        for p in &mut t3[..len] {
            if p.0 == 0x608 {
                p.1 = l.ta_regions as u32;
            }
        }
        for &(off, v) in over {
            if let Some(p) = t3[..len - 1].iter_mut().find(|p| p.0 == off) {
                p.1 = v;
            }
        }
        out.done = run(e, &t3[..len], RENDER_DONE, 200_000);
        if !out.done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("    (render timed out: core reset, memory manager ready again: {ok})");
        }
        out
    }

    /// M: the second texture unit (the performance gate's P5): two flat
    /// 8 x 8 grey textures (code 10), 0x80 in unit 0 and 0x40 in unit 1,
    /// through the TA with the state block's mask bit 3 (unit 1's three
    /// words after unit 0's) and vertices of 9 words (u, v for each unit).
    /// Unit 1 modulating gives about 0x20; ignored, 0x80. A one-unit draw
    /// first (the control). The object the TA wrote is logged: the direct
    /// form's layout for two units.
    #[inline(never)]
    fn two_units(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        for k in 0..16 {
            let _ = store32_volatile(vram, TEX + 4 * k, 0x8080_8080);
            let _ = store32_volatile(vram, TEX_B + 4 * k, 0x4040_4040);
        }
        cpu::drain_write_buffer();
        let tex = |addr: usize, tex2: u32| {
            let mut w = m3::ta::dell_texture(addr as u32);
            w[0] = m3::tex::tex0(0, 10);
            w[2] = tex2;
            w
        };
        // Unit 0 as test L (texture's grey and alpha); unit 1 MODULATE (the
        // GLES table's unit-1 form: bits 11 and 7 set).
        let a = tex(TEX, 0x8608_4610);
        let clip = m3::ta::clip(W, H);
        let corners = [
            (100.0f32, 100.0f32, 0.0f32, 0.0f32),
            (380.0, 100.0, 1.0, 0.0),
            (100.0, 540.0, 0.0, 1.0),
        ];
        // Version 2 (2026-10-22): mask bit 3 alone left the TA reading
        // 7-word vertices. TSP bits 19-17 (1 in Dell's textured draw, 2 in
        // DoomGLES's) and bit 16 (set by the GLES driver from a second
        // field) are the candidates for "texture coordinate sets".
        let dell = m3::ta::TSP_DELL_TEXTURED & !(0xF << 16);
        // Version 2 found TSP bit 16 = a second coordinate set (the TA
        // built two-unit objects, list layout 12) but each such render hung,
        // and every render after the first hang hung too (the recovery, not
        // the case). Version 3: bit 16 with unit 1's combine word in three
        // forms (the GLES table's unit-1 MODULATE, unit 0's form, a unit-1
        // REPLACE), a one-unit control after each, to tell a case's hang
        // from a recovery that didn't.
        let two = dell | 1 << 17 | 1 << 16;
        let control = (
            "one unit (control)",
            1usize,
            m3::ta::TSP_DELL_TEXTURED,
            0u32,
        );
        for (name, units, tsp, tex2b) in [
            control,
            ("two units, unit 1 TEX2 0x86084e10", 2, two, 0x8608_4E10),
            control,
            ("two units, unit 1 TEX2 0x86084610", 2, two, 0x8608_4610),
            control,
            ("two units, unit 1 TEX2 0xd6087e10", 2, two, 0xD608_7E10),
            control,
        ] {
            let b = tex(TEX_B, tex2b);
            rt::check_in();
            let fill = rp_image::need(
                cmd::fill(
                    Format::BPP32,
                    TARGET,
                    STRIDE,
                    (0, 0, W as u16, H as u16),
                    GREY,
                ),
                "a 2D rectangle",
            );
            let _ = e.submit(&fill).and_then(|()| e.finish(vram, 200_000));
            let mut st = [0u32; 48];
            let mut n = 0;
            let mut put = |v: u32| {
                st[n] = v;
                n += 1;
            };
            put(if units == 2 { 0x1000_005F } else { 0x1000_0057 });
            put(tsp);
            put(ISP_ALWAYS);
            a.iter().for_each(|&v| put(v));
            if units == 2 {
                b.iter().for_each(|&v| put(v));
            }
            put(clip);
            put(0);
            put(m3::ta::STRIP);
            for &(x, y, u, v) in &corners {
                for word in [
                    x.to_bits(),
                    y.to_bits(),
                    1.0f32.to_bits(),
                    1.0f32.to_bits(),
                    0xFFFF_FFFF,
                    u.to_bits(),
                    v.to_bits(),
                ] {
                    put(word);
                }
                if units == 2 {
                    put(u.to_bits());
                    put(v.to_bits());
                }
            }
            let TaOut { ta, done, .. } = through_ta(e, vram, &[&st[..n]], 32, &[]);
            log!(
                "  M {name} (TSP 0x{tsp:08x}): {n} words, TA done {ta}, render done {done}; inside (150,200) 0x{:08x}, outside (400,600) 0x{:08x}",
                px(vram, 150, 200),
                px(vram, 400, 600)
            );
        }
    }

    /// `lines` on the label (upper case; each cut to the screen's width),
    /// the rest of its text area blank.
    #[inline(never)]
    fn say(vram: &mut [u8], lines: &[&str]) {
        let cell = (WIDTH + 1) * LABEL_SCALE;
        let mut row = [0u32; LABEL_STRIDE / 4];
        for l in 0..LABEL_LINES {
            let text = lines.get(l).copied().unwrap_or("");
            for py in 0..LABEL_LINE_H {
                row.fill(PAPER | PAPER << 16);
                let gy = py / LABEL_SCALE;
                if gy < font::HEIGHT {
                    for (k, ch) in text.chars().take(480 / cell).enumerate() {
                        let bits = font::glyph(ch.to_ascii_uppercase())[gy];
                        for gx in 0..WIDTH {
                            if bits & (1 << (WIDTH - 1 - gx)) == 0 {
                                continue;
                            }
                            for dx in 0..LABEL_SCALE {
                                let x = 8 + k * cell + gx * LABEL_SCALE + dx;
                                let shift = 16 * (x % 2);
                                row[x / 2] = (row[x / 2] & !(0xFFFF << shift)) | INK << shift;
                            }
                        }
                    }
                }
                let y = LABEL_TOP + l * LABEL_LINE_H + py;
                store(vram, LABEL + y * LABEL_STRIDE, &row);
            }
        }
        cpu::drain_write_buffer();
    }

    /// The label plane cleared and shown; the display's registers as they
    /// were.
    fn show_label(regs: &mut Mmio, vram: &mut [u8]) -> [(usize, u32); 5] {
        for k in 0..LABEL_STRIDE * H as usize / 4 {
            let _ = store32_volatile(vram, LABEL + 4 * k, PAPER | PAPER << 16);
        }
        cpu::drain_write_buffer();
        show(
            regs,
            Plane {
                format: PixelFormat::Rgb565,
                width: W as u16,
                height: H as u16,
                offset: LABEL,
                stride: LABEL_STRIDE,
            },
        )
    }

    /// The display's output signature (mbxfb's `DSIG`): 0 when nothing is
    /// sent, and for an all-black picture (run N), which the label never is.
    const DSIG: usize = 0x2184;
    /// `DINTRS` bit 4: the graphics stream's underrun; set, the display
    /// stops sending until it is cleared (run G4's dark display, 2700g.md).
    const GFX_UNDERRUN: u32 = 1 << 4;
    /// A frame at 60 Hz, rounded up.
    const FRAME_US: u32 = 16_700;

    /// The display after a clock change (quality audit C69): the underrun
    /// flag cleared Dell's way (read, written back without it), then the
    /// output signature two frames on and a frame after that. If nothing is
    /// sent, the label plane is set again and read once more. Logged with
    /// `when`; true if the display sends.
    fn display_check(regs: &mut Mmio, when: &str) -> bool {
        let s = rp_drv_mbx_display::ack_events(regs, GFX_UNDERRUN);
        rt::delay_us(2 * FRAME_US);
        let a = regs.read32(DSIG);
        rt::delay_us(FRAME_US);
        let b = regs.read32(DSIG);
        let mut sending = a != 0 && b != 0;
        let _ = write!(
            RunLog,
            "    display {when}: underrun {}, DSIG 0x{a:x} then 0x{b:x}",
            s & GFX_UNDERRUN != 0
        );
        if !sending {
            let _ = show(
                regs,
                Plane {
                    format: PixelFormat::Rgb565,
                    width: W as u16,
                    height: H as u16,
                    offset: LABEL,
                    stride: LABEL_STRIDE,
                },
            );
            let _ = rp_drv_mbx_display::ack_events(regs, GFX_UNDERRUN);
            rt::delay_us(2 * FRAME_US);
            let c = regs.read32(DSIG);
            sending = c != 0;
            let _ = write!(RunLog, "; the label plane set again: DSIG 0x{c:x}");
        }
        log!("; {}", if sending { "sending" } else { "NOT SENDING" });
        sending
    }

    /// Windows Mobile's clock and bus timing back, wherever test G left
    /// them (no state kept, so the crash path can call it): `rp-gpubench`'s
    /// `clock_restore`, which every clock run has used. The PLL is always
    /// written again.
    fn clock_back<B: RegisterBus>(regs: &mut B) {
        let _ = diag::set_cs3_vlio(CS3_LONG.0, CS3_LONG.1, CS3_LONG.2);
        let clk = regs.read32(MBXCLK);
        clock::restore(regs, (clk & 3) | 1 << 2);
        rt::delay_us(1_000);
        if let Some(a) = clock::set_speed(regs, Speed::WM, rt::delay_us)
            && a.rewrites + a.unwritten > 0
        {
            log!(
                "    put back: {} memory-controller writes written again, {} never held",
                a.rewrites,
                a.unwritten
            );
        }
        let _ = diag::set_cs3_vlio(CS3_WM.0, CS3_WM.1, CS3_WM.2);
        // G5 may have set NINTPW by Dell's rule: Windows Mobile's back
        // (the same rule at its clock: 28, as read under Dell).
        if TEST_G5 || TEST_G11 {
            regs.write32(super::NINTPW, super::nintpw(Speed::WM.khz()));
        }
        // G11: CS2's half Windows Mobile's again.
        if TEST_G11 {
            let _ = diag::set_cs2_half(super::WM_CS2);
        }
    }

    /// `speed` (a core PLL and its memory timing) with the cores at divide
    /// 1, as `rp-gpubench`'s `clock_apply` steps to it: the long bus timing,
    /// the cores at divide 2, the PLL and its memory timing, then divide 1
    /// and the bus timing of the clock's tier (Dell's: the long one below
    /// 99 MHz). False if the PLL didn't lock or a memory-controller write
    /// never held (Windows Mobile's clock is then back).
    ///
    /// ADR-0035's rules this follows: 3, CL2 always (callers pass
    /// `Speed::ladder`; G3's first run took Dell's CL3 and rendered nothing
    /// at 133.25 MHz); 4, every memory-controller write read back and the
    /// step failed if one never holds (`clock::set_speed`); 8, every change
    /// followed by the underrun check and Dell's clear (the callers'
    /// `display_check`, quality audit C69).
    ///
    /// With `dell_order` (test G5), the last divider write Dell's way: both
    /// MBX clocks off (`MBXCLK` 0) just before it (`ddi_2700g`
    /// `0x02120370`; docs/hw/2700g.md "Dell's register writes").
    fn to_divide_1<B: RegisterBus>(regs: &mut B, speed: Speed, dell_order: bool) -> bool {
        let _ = diag::set_cs3_vlio(CS3_LONG.0, CS3_LONG.1, CS3_LONG.2);
        let clk = regs.read32(MBXCLK);
        clock::restore(regs, (clk & 3) | 1 << 2);
        rt::delay_us(1_000);
        let ok = match clock::set_speed(regs, speed, rt::delay_us) {
            Some(a) => {
                if a.rewrites + a.unwritten > 0 {
                    log!(
                        "    memory-controller writes: {} written again, {} never held",
                        a.rewrites,
                        a.unwritten
                    );
                }
                a.unwritten == 0
            }
            None => false,
        };
        if !ok {
            clock_back(regs);
            return false;
        }
        let clk = regs.read32(MBXCLK);
        if dell_order {
            regs.write32(MBXCLK, 0);
        }
        clock::restore(regs, clk & 3);
        rt::delay_us(1_000);
        let cs3 = if speed.khz() < 99_000 {
            CS3_LONG
        } else {
            CS3_WM
        };
        let _ = diag::set_cs3_vlio(cs3.0, cs3.1, cs3.2);
        // G11: CS2's half as Dell's driver sets it at divide 1.
        if G11_CS2.with(|c| *c) {
            let _ = diag::set_cs2_half(super::DELL_CS2_DIV1);
        }
        true
    }

    /// The clock registers as the 2700G holds them.
    fn clock_readback(regs: &mut Mmio) {
        log!(
            "    read back: COREPLL 0x{:x} MBXCLK 0x{:x} LMTYPE 0x{:x} LMTIM 0x{:x} LMREFRESH 0x{:x}",
            regs.read32(clock::COREPLL),
            regs.read32(MBXCLK),
            regs.read32(clock::LMTYPE),
            regs.read32(clock::LMTIM),
            regs.read32(clock::LMREFRESH)
        );
    }

    /// On a crash, first: Windows Mobile's clock and bus timing back.
    fn put_back() {
        if let Some((mut regs, _)) = rp_hal_x51v::devices::emergency_mbx() {
            clock_back(&mut regs);
        }
    }

    /// A clean start (GC2's rule 5): the 3D core reset and its memory
    /// manager set up again.
    fn clean(e: &mut Engine<Mmio>, vram: &mut [u8]) -> bool {
        e.reset(rt::delay_us);
        pm_init(e, vram)
    }

    /// The target filled with [`SENTINEL`] by the CPU (not the 2D engine,
    /// which runs on the clock under test).
    fn blank(vram: &mut [u8]) {
        let target = lay().target;
        for k in 0..target_words() {
            let _ = store32_volatile(vram, target + 4 * k, SENTINEL);
        }
        cpu::drain_write_buffer();
    }

    /// The target held to the picture at `reference`.
    fn compare(vram: &[u8], reference: usize) -> Tally {
        let mut t = Tally::default();
        let (target, per) = (lay().target, per_word());
        // RGB565 (G6): a word is two pixels, placed at its first; a step
        // read as ARGB8888's means nothing there.
        for i in 0..target_words() as u32 {
            let at = 4 * i as usize;
            let p = i * per;
            t.add_px(
                (p % W, p / W),
                (W, H),
                load32_volatile(vram, reference + at).unwrap_or(0),
                load32_volatile(vram, target + at).unwrap_or(0xDEAD_BEEF),
            );
        }
        t
    }

    /// The picture at `at` hashed (FNV-1a over its words).
    fn picture_hash(vram: &[u8], at: usize) -> u32 {
        let mut hash = 0x811C_9DC5u32;
        for k in 0..target_words() {
            let v = load32_volatile(vram, at + 4 * k).unwrap_or(0);
            hash = (hash ^ v).wrapping_mul(0x0100_0193);
        }
        hash
    }

    /// The target copied to [`REF`].
    fn keep_reference(vram: &mut [u8]) {
        let target = lay().target;
        for k in 0..target_words() {
            let v = load32_volatile(vram, target + 4 * k).unwrap_or(0);
            let _ = store32_volatile(vram, REF + 4 * k, v);
        }
        cpu::drain_write_buffer();
    }

    /// Test G's three ways (and a fourth after them): how each draws.
    #[derive(Clone, Copy)]
    enum Way {
        /// Through the TA: Dell's clear, then each triangle its own strip.
        Ta,
        /// Our direct objects: a black background, the two Gouraud
        /// triangles, every list word ours (layout 4).
        Ours,
        /// The same objects with the list word the TA wrote for its
        /// Gouraud object ([`adopt_flags`]).
        OursTaFlags(u32),
        /// Through the TA: the clear, then the quad as one strip of four
        /// (as a GLES program draws a quad).
        TaStrip4,
        /// Test G2: our direct path with the TA's choices taken one at a
        /// time.
        Direct(Form),
        /// Test G3: Dell's sequence through the TA as `glClear` then
        /// `glDrawArrays(GL_TRIANGLE_STRIP)` send it: the colour clear
        /// ([`super::dell_clear_draw`]), then the quad as one strip of four.
        TaDell,
        /// Test G5: our form (two Gouraud triangles over our giant flat
        /// background) with the items of [`super::DELL_ITEMS`] in the mask
        /// copied from Dell's set-up.
        Dell(u32),
        /// Test G6: ours in [`super::G6_LAYOUTS`]'s layout k (k 0-3), Dell's
        /// whole state with it from k 2, through the TA at k 3.
        G6(u8),
        /// Test G7: Dell's frame replayed word for word (with his NINTPW
        /// rule and clock order when true).
        Frame(bool),
        /// Test G8: [`super::G8_WAYS`]'s way k (ours or Dell's frame, and
        /// how the render is waited for).
        G8(u8),
        /// Test G10: [`super::G10_WAYS`]'s way k.
        G10(u8),
        /// Test G11: [`super::G11_WAYS`]'s way k.
        G11(u8),
        /// Test G12: [`super::G12_WAYS`]'s way k.
        G12(u8),
        /// Test G13: [`super::G13_WAYS`]'s way k.
        G13(u8),
    }

    /// Test G2's direct forms: which of the TA's choices it takes.
    #[derive(Clone, Copy)]
    struct Form {
        name: &'static str,
        /// One four-corner object (layout 5, [`super::quad_object`]), not
        /// two triangles.
        quad: bool,
        /// z 1.0 (the TA's), not 0.
        z1: bool,
        /// What lies under the quad.
        bg: Bg,
        /// Each tile lists only the objects that touch it (the TA's lists),
        /// not every object.
        tiled: bool,
        /// Start-table words written instead of ours (test G4; not with
        /// `tiled`).
        regs: &'static [(usize, u32)],
    }

    /// Backgrounds (all black and opaque but the clear's, which has no
    /// colour): G2 found ours behind the divide-1 errors and the clear's
    /// colourless object free of them; G3 finds which of ours's traits it
    /// is.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Bg {
        /// Ours: one flat triangle (Dell's TSP) to (2048, 2048), z 0.
        Giant,
        /// The same at z 1.0.
        GiantZ1,
        /// The GLES clear's colourless object (layout 0; G2's way 4).
        Clear,
        /// Dell's colour clear by hand: one flat four-corner object over
        /// the screen exactly, z 1.0 ([`super::dell_clear_corners`]).
        DellQuad,
        /// Two flat triangles over the screen exactly, z 0 (the size alone).
        ScreenTris,
        /// One Gouraud triangle to (2048, 2048), every corner black.
        Gouraud,
        /// None.
        Nothing,
    }

    impl Way {
        const fn name(self) -> &'static str {
            match self {
                Self::Ta => "through the TA",
                Self::Ours => "our objects, our layout 4",
                Self::OursTaFlags(_) => "our objects, the TA's list flags",
                Self::TaStrip4 => "through the TA, one strip of four",
                Self::Direct(f) => f.name,
                Self::TaDell => "Dell's clear and strip, through the TA",
                Self::Dell(super::DELL_ALL) => "ours with Dell's whole state",
                Self::Dell(_) => "ours with part of Dell's state",
                Self::G6(0) => "ours, our places",
                Self::G6(1) => "Dell's parameter memory (8 MiB at 0x22b000)",
                Self::G6(2) => "Dell's places and state (RGB565 target)",
                Self::G6(_) => "Dell's places and state, through the TA",
                Self::Frame(false) => "Dell's frame replayed",
                Self::Frame(true) => "Dell's frame replayed, his NINTPW and clock order",
                Self::G8(0) => "ours, the status read without a pause (as before)",
                Self::G8(1) => "ours, the chip left alone, the status read once after",
                Self::G8(2) => "ours, the status read once a millisecond",
                Self::G8(3) => "Dell's frame, the chip left alone, the status read once after",
                Self::G8(_) => "Dell's frame, the status read once a millisecond",
                Self::G10(0) => "ours as before: written, rendered and read at divide 1",
                Self::G10(1) => {
                    "ours: written and read at Windows Mobile's clock, rendered at divide 1"
                }
                Self::G10(2) => {
                    "Dell's frame: written and read at Windows Mobile's clock, rendered at divide 1"
                }
                Self::G10(_) => {
                    "ours: written and rendered at divide 1, read at Windows Mobile's clock"
                }
                Self::G11(0) => "ours, CS2 as found",
                Self::G11(1) => "ours, CS2 Dell's at divide 1",
                Self::G11(2) => "Dell's frame, CS2 Dell's at divide 1",
                Self::G11(_) => "Dell's frame, CS2, NINTPW and clock order Dell's",
                Self::G12(0) => "ours as before",
                Self::G12(1) => "Dell's frame as before",
                Self::G12(2) => "Dell's frame, the CPU silent while it renders",
                Self::G12(3) => "Dell's frame, the loader-only clocks off while it renders",
                Self::G12(_) => "Dell's frame, the CPU silent and the loader-only clocks off",
                Self::G13(0) => "ours, the display at the loader's 60 Hz",
                Self::G13(1) => "Dell's frame, the display at the loader's 60 Hz",
                Self::G13(2) => "ours, the display at Windows Mobile's 56 Hz",
                Self::G13(_) => "Dell's frame, the display at Windows Mobile's 56 Hz",
            }
        }
    }

    /// Where test G2's per-tile lists go: after the objects, in the
    /// parameter memory (32 bytes a tile: up to seven objects and the end).
    const LISTS: usize = PARAMS + 0x1000;
    const LIST_WORDS: usize = 8;
    const _: () = assert!(LISTS + 4 * LIST_WORDS * (COLS * ROWS) as usize <= PM + PM_SIZE as usize);

    /// Which of [`TRIS`] an object covers (bit k: triangle k); every
    /// tile for [`ALL`].
    const ALL: u8 = 0xFF;

    /// `objs` (words, list flags, cover) rendered with a list for each
    /// tile holding only the objects that touch it ([`super::touches`]);
    /// true if it finished. A timeout resets the core.
    #[inline(never)]
    fn scene_tiled(e: &mut Engine<Mmio>, vram: &mut [u8], objs: &[(&[u32], u32, u8)]) -> bool {
        let mut at = OBJECT;
        let mut word = [m3::END; LIST_WORDS - 1];
        let n = objs.len().min(LIST_WORDS - 1);
        for (k, (w, flags, _)) in objs[..n].iter().enumerate() {
            store(vram, at, w);
            word[k] = flags | ((at - PARAMS) / 4) as u32;
            at += 4 * w.len();
        }
        let p = |k: usize| (QUAD[k].0, QUAD[k].1);
        let total = COLS * ROWS;
        for t in 0..total {
            let (c, r) = (t % COLS, t / COLS);
            let rect = (
                (c * m3::TILE_W) as f32,
                (r * m3::TILE_H) as f32,
                ((c + 1) * m3::TILE_W) as f32,
                ((r + 1) * m3::TILE_H) as f32,
            );
            let mut list = [m3::END; LIST_WORDS];
            let mut len = 0;
            for (k, &(_, _, cover)) in objs[..n].iter().enumerate() {
                let hit = cover == ALL
                    || (0..TRIS.len())
                        .any(|j| cover & 1 << j != 0 && super::touches(TRIS[j].map(p), rect));
                if hit {
                    list[len] = word[k];
                    len += 1;
                }
            }
            let addr = LISTS + 4 * LIST_WORDS * t as usize;
            store(vram, addr, &list);
            let entry = m3::region_entry(c, r, addr as u32, t + 1 == total);
            store(vram, REGIONS + 8 * t as usize, &entry);
        }
        cpu::drain_write_buffer();
        let (t, len) = table();
        let done = run(e, &t[..len], RENDER_DONE, 200_000);
        if !done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("  (timed out: core reset, memory manager ready again: {ok})");
        }
        done
    }

    /// Test G2's and G3's direct form `f` drawn (the target already
    /// blanked); with `quad` false, its background alone. With `words`, the
    /// quad's first object, its list word and
    /// the background's logged.
    fn draw_direct(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        f: Form,
        words: usize,
        quad_too: bool,
        over: &[(usize, u32)],
    ) -> bool {
        let z = if f.z1 { 1.0 } else { 0.0 };
        const BLACK: u32 = 0xFF00_0000;
        let giant = |tsp: u32, z: f32| {
            m3::triangle(
                ISP_ALWAYS,
                tsp,
                &[(0.0, 0.0), (2048.0, 0.0), (0.0, 2048.0)].map(|(x, y)| Vertex {
                    x,
                    y,
                    z,
                    rhw: 1.0,
                    argb: BLACK,
                }),
            )
        };
        let screen = |t: [usize; 3]| {
            m3::triangle(
                ISP_ALWAYS,
                TSP_DELL,
                &t.map(|k| Vertex::flat(QUAD[k].0, QUAD[k].1, BLACK)),
            )
        };
        let (flat0, flat1, smooth) = (
            giant(TSP_DELL, 0.0),
            giant(TSP_DELL, 1.0),
            giant(TSP_SMOOTH, 0.0),
        );
        let (half0, half1) = (screen(TRIS[0]), screen(TRIS[1]));
        let clear_bg = super::clear_object(W, H);
        let dell_bg = super::four_corner_object(
            ISP_ALWAYS,
            super::DELL_CLEAR_TSP,
            1.0,
            super::dell_clear_corners(W, H, BLACK),
        );
        let mut objs: [(&[u32], u32, u8); 4] = [(&[], 0, 0); 4];
        let mut n = 0;
        let bgs: [(&[u32], u32, u8); 2] = match f.bg {
            Bg::Giant => [(&flat0, STRIP, ALL), (&[], 0, 0)],
            Bg::GiantZ1 => [(&flat1, STRIP, ALL), (&[], 0, 0)],
            Bg::Clear => [(&clear_bg, super::CLEAR_FLAGS, ALL), (&[], 0, 0)],
            Bg::DellQuad => [(&dell_bg, super::QUAD_FLAGS, ALL), (&[], 0, 0)],
            Bg::ScreenTris => [(&half0, STRIP, 1), (&half1, STRIP, 2)],
            Bg::Gouraud => [(&smooth, STRIP, ALL), (&[], 0, 0)],
            Bg::Nothing => [(&[], 0, 0), (&[], 0, 0)],
        };
        for o in bgs {
            if !o.0.is_empty() {
                objs[n] = o;
                n += 1;
            }
        }
        let tri = |t: [usize; 3]| {
            m3::triangle(
                ISP_ALWAYS,
                TSP_SMOOTH,
                &t.map(|k| Vertex {
                    x: QUAD[k].0,
                    y: QUAD[k].1,
                    z,
                    rhw: 1.0,
                    argb: QUAD[k].2,
                }),
            )
        };
        let (a, b) = (tri(TRIS[0]), tri(TRIS[1]));
        let quad = super::quad_object(ISP_ALWAYS, TSP_SMOOTH, z);
        let first = n;
        if !quad_too {
        } else if f.quad {
            objs[n] = (&quad, super::QUAD_FLAGS, 3);
            n += 1;
        } else {
            objs[n] = (&a, STRIP, 1);
            objs[n + 1] = (&b, STRIP, 2);
            n += 2;
        }
        if words > 0 && first < n {
            let _ = write!(
                RunLog,
                "    list word 0x{:08x} (layout {}), object:",
                objs[first].1,
                (objs[first].1 & super::LAYOUT) >> 21
            );
            for w in objs[first].0 {
                let _ = write!(RunLog, " {w:08x}");
            }
            let _ = write!(RunLog, "; background:");
            for o in &objs[..first] {
                let _ = write!(RunLog, " list word 0x{:08x}, {} words", o.1, o.0.len());
                for w in o.0.iter().take(2) {
                    let _ = write!(RunLog, " {w:08x}");
                }
                let _ = write!(RunLog, ";");
            }
            log!("");
        }
        if f.tiled {
            scene_tiled(e, vram, &objs[..n])
        } else {
            let mut plain = [(&[][..], 0u32); 4];
            for (k, o) in objs[..n].iter().enumerate() {
                plain[k] = (o.0, o.1);
            }
            scene_with(
                e,
                vram,
                &plain[..n],
                if over.is_empty() { f.regs } else { over },
            )
        }
    }

    /// One render of `way` into the target (blanked first). With `words`,
    /// the TA's lists and objects are logged. Whether it finished, the
    /// render's time in microseconds, and the TA's first tile's list.
    fn draw(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        way: Way,
        words: usize,
    ) -> (bool, u32, [u32; TA_LIST]) {
        blank(vram);
        let clip = m3::ta::clip(W, H);
        let t0 = rt::ticks();
        match way {
            Way::Ta | Way::TaStrip4 => {
                let clear = m3::ta::clear_stream(W, H);
                let (a, na) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &TRIS[0]);
                let (b, nb) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &TRIS[1]);
                let (q, nq) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &[0, 1, 3, 2]);
                let out = if let Way::Ta = way {
                    through_ta(e, vram, &[&clear[..], &a[..na], &b[..nb]], words, &[])
                } else {
                    through_ta(e, vram, &[&clear[..], &q[..nq]], words, &[])
                };
                (out.ta && out.done, us_since(t0), out.list)
            }
            Way::Ours | Way::OursTaFlags(_) => {
                let flags = match way {
                    Way::OursTaFlags(f) => f,
                    _ => STRIP,
                };
                let bg = m3::triangle(
                    ISP_ALWAYS,
                    TSP_DELL,
                    &[
                        Vertex::flat(0.0, 0.0, 0xFF00_0000),
                        Vertex::flat(2048.0, 0.0, 0xFF00_0000),
                        Vertex::flat(0.0, 2048.0, 0xFF00_0000),
                    ],
                );
                let tri = |t: [usize; 3]| {
                    m3::triangle(
                        ISP_ALWAYS,
                        TSP_SMOOTH,
                        &t.map(|k| Vertex::flat(QUAD[k].0, QUAD[k].1, QUAD[k].2)),
                    )
                };
                let (a, b) = (tri(TRIS[0]), tri(TRIS[1]));
                if words > 0 {
                    let _ = write!(
                        RunLog,
                        "    our list word 0x{flags:08x} (layout {}), object:",
                        (flags & super::LAYOUT) >> 21
                    );
                    for w in a {
                        let _ = write!(RunLog, " {w:08x}");
                    }
                    log!("");
                }
                let done = scene(e, vram, &[(&bg, STRIP), (&a, flags), (&b, flags)]);
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::Direct(f) => {
                let done = draw_direct(e, vram, f, words, true, &[]);
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::Dell(mask) => {
                let mut over = [(0usize, 0u32); 16];
                let n = dell_words(e, mask, &mut over);
                let done = draw_direct(e, vram, G5_OURS, words, true, &over[..n]);
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::TaDell => {
                let clear = super::dell_clear_draw(ISP_ALWAYS, W, H, 1.0, 0xFF00_0000);
                let (q, nq) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &[0, 1, 3, 2]);
                let out = through_ta(e, vram, &[&clear[..], &q[..nq]], words, &[]);
                (out.ta && out.done, us_since(t0), out.list)
            }
            Way::Frame(_) => (replay(e, vram), us_since(t0), [m3::END; TA_LIST]),
            Way::G13(k) => {
                let done = if super::G13_WAYS[k as usize % super::G13_WAYS.len()].0 {
                    replay(e, vram)
                } else {
                    draw_direct(e, vram, G5_OURS, words, true, &[])
                };
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::G12(k) => {
                let done = if super::G12_WAYS[k as usize % super::G12_WAYS.len()].0 {
                    replay(e, vram)
                } else {
                    draw_direct(e, vram, G5_OURS, words, true, &[])
                };
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::G11(k) => {
                let done = if super::G11_WAYS[k as usize % super::G11_WAYS.len()].0 {
                    replay(e, vram)
                } else {
                    draw_direct(e, vram, G5_OURS, words, true, &[])
                };
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::G10(k) => {
                let (dell, mode) = super::G10_WAYS[k as usize % super::G10_WAYS.len()];
                let pll = G10_PLL.with(|p| p.0);
                let done = match (mode, pll) {
                    (super::G10Mode::AsBefore, _) | (_, 0) => {
                        if dell {
                            replay(e, vram)
                        } else {
                            draw_direct(e, vram, G5_OURS, words, true, &[])
                        }
                    }
                    (super::G10Mode::RenderOnly, pll) => {
                        let (pairs, n) = if dell {
                            prepare_frame(vram)
                        } else {
                            prepare_ours(vram)
                        };
                        g10_change(e, Some(pll));
                        let done = start(e, vram, &pairs[..n]);
                        g10_change(e, None);
                        done
                    }
                    (super::G10Mode::ReadAtWm, pll) => {
                        g10_change(e, Some(pll));
                        let done = draw_direct(e, vram, G5_OURS, words, true, &[]);
                        g10_change(e, None);
                        done
                    }
                };
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::G8(k) => {
                let done = if super::G8_WAYS[k as usize % super::G8_WAYS.len()].0 {
                    replay(e, vram)
                } else {
                    draw_direct(e, vram, G5_OURS, words, true, &[])
                };
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            // G6: ours in the way's layout; ways 3 and 4 with Dell's whole
            // state (G5's items), way 4 through the TA as Dell's driver
            // sends it (his colour clear, then the quad as one strip).
            Way::G6(k) => {
                let mut over = [(0usize, 0u32); 16];
                let n = if k >= 2 {
                    dell_words(e, super::DELL_ALL, &mut over)
                } else {
                    0
                };
                if k == 3 {
                    let clear = super::dell_clear_draw(ISP_ALWAYS, W, H, 1.0, 0xFF00_0000);
                    let (q, nq) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &[0, 1, 3, 2]);
                    let out = through_ta(e, vram, &[&clear[..], &q[..nq]], words, &over[..n]);
                    (out.ta && out.done, us_since(t0), out.list)
                } else {
                    let done = draw_direct(e, vram, G5_OURS, words, true, &over[..n]);
                    (done, us_since(t0), [m3::END; TA_LIST])
                }
            }
        }
    }

    /// The list word of the TA's Gouraud object in `list` (its object's
    /// first two words hold [`TSP_SMOOTH`]), if it wrote one.
    fn gouraud_word(vram: &[u8], list: &[u32; TA_LIST]) -> Option<u32> {
        list.iter()
            .take_while(|&&p| p != m3::END)
            .copied()
            .find(|&p| {
                let obj = PARAMS + 4 * (p & 0x001F_FFFF) as usize;
                (0..2).any(|j| load32_volatile(vram, obj + 4 * j) == Some(TSP_SMOOTH))
            })
    }

    /// What one way gave at one setting.
    #[derive(Default)]
    struct Setting {
        /// The clock reached it.
        clocked: bool,
        /// Renders at divide 1 equal to the reference.
        right: u32,
        /// Renders at divide 1 that never finished.
        unfinished: u32,
        /// Every wrong word at divide 1.
        tally: Tally,
        /// Wrong renders whose second read (50 ms later) differed.
        unsettled: u32,
        /// The fastest render at divide 1, microseconds.
        us: u32,
        /// The display sent at every check of this setting (after the
        /// change, after the draws, back at Windows Mobile's clock).
        screen: bool,
        /// The control at Windows Mobile's clock after: equal.
        control: bool,
    }

    /// The most settings a way runs at.
    const SETTINGS: usize = 3;

    /// What one way gave.
    #[derive(Default)]
    struct WayResult {
        /// The reference rendered twice the same.
        repeatable: bool,
        /// The reference's fastest render, microseconds.
        us_wm: u32,
        /// Each setting's result, in the order run.
        at: [Setting; SETTINGS],
    }

    impl WayResult {
        /// Renders right at setting `k`, if it ran.
        fn right(&self, k: usize) -> Option<u32> {
            (self.repeatable && self.at[k].clocked).then_some(self.at[k].right)
        }
    }

    /// One way: its reference at Windows Mobile's clock (rendered twice,
    /// equal; kept), then at each of `settings` (a core PLL with the cores
    /// at divide 1, and its name in MHz) 20 renders held to it and a control
    /// at Windows Mobile's clock. `step` and `steps` label the screen.
    #[inline(never)]
    fn way(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        w: Way,
        settings: &[(u32, &str)],
        step: u32,
        steps: u32,
    ) -> (WayResult, [u32; TA_LIST]) {
        let mut r = WayResult::default();
        let mut head = [0u8; 32];
        let head = fmt(&mut head, format_args!("STEP {step} OF {steps}"));
        let mut name = [0u8; 32];
        let name = fmt(&mut name, format_args!("WAY {}: {}", step - 1, w.name()));
        say(vram, &[G_TITLE, head, name, "WM CLOCK: REFERENCE"]);
        set_lay(match w {
            Way::G6(k) => super::G6_LAYOUTS[k as usize % super::G6_LAYOUTS.len()],
            Way::Frame(_) => frame()
                .as_ref()
                .and_then(super::frame_layout)
                .unwrap_or(super::OURS_LAYOUT),
            Way::G13(k) if super::G13_WAYS[k as usize % super::G13_WAYS.len()].0 => frame()
                .as_ref()
                .and_then(super::frame_layout)
                .unwrap_or(super::OURS_LAYOUT),
            Way::G12(k) if super::G12_WAYS[k as usize % super::G12_WAYS.len()].0 => frame()
                .as_ref()
                .and_then(super::frame_layout)
                .unwrap_or(super::OURS_LAYOUT),
            Way::G11(k) if super::G11_WAYS[k as usize % super::G11_WAYS.len()].0 => frame()
                .as_ref()
                .and_then(super::frame_layout)
                .unwrap_or(super::OURS_LAYOUT),
            Way::G10(k) if super::G10_WAYS[k as usize % super::G10_WAYS.len()].0 => frame()
                .as_ref()
                .and_then(super::frame_layout)
                .unwrap_or(super::OURS_LAYOUT),
            Way::G8(k) if super::G8_WAYS[k as usize % super::G8_WAYS.len()].0 => frame()
                .as_ref()
                .and_then(super::frame_layout)
                .unwrap_or(super::OURS_LAYOUT),
            _ => super::OURS_LAYOUT,
        });
        // G13: the display's pixel clock for this way, its lock awaited and
        // the display checked (ADR-0035 rule 8 for any clock).
        if let Way::G13(k) = w {
            let want = if super::G13_WAYS[k as usize % super::G13_WAYS.len()].1 {
                rp_drv_mbx_display::pll::WM
            } else {
                rp_drv_mbx_display::pll::SIXTY
            };
            set_display_pll(&mut e.regs, want);
        }
        let g12 = match w {
            Way::G12(k) => super::G12_WAYS[k as usize % super::G12_WAYS.len()],
            _ => (false, false, false),
        };
        G12_CLOCKS.with(|c| *c = g12.2);
        G11_CS2.with(|c| {
            *c = match w {
                Way::G11(k) => super::G11_WAYS[k as usize % super::G11_WAYS.len()].1,
                _ => false,
            }
        });
        WAIT.with(|v| {
            *v = match w {
                Way::G8(k) => super::G8_WAYS[k as usize % super::G8_WAYS.len()].1,
                Way::G12(_) if g12.1 => super::Wait::Silent,
                _ => super::Wait::Tight,
            }
        });
        log!("{G_NAME} way {} ({}):", step - 1, w.name());
        rt::check_in();
        let _ = display_check(&mut e.regs, "before the reference");
        let ready = clean(e, vram);
        // G3 and G4: the background alone first. The quad covers the whole target,
        // so a background never shows in the reference; its own picture
        // proves it drew (and ways whose backgrounds must look alike are
        // held to each other).
        let mut bg_hash = None;
        if let (true, Way::Direct(f)) = (TEST_G3 || TEST_G4, w)
            && f.bg != Bg::Nothing
        {
            blank(vram);
            let done = draw_direct(e, vram, f, 0, false, &[]);
            let h = picture_hash(vram, lay().target);
            bg_hash = Some(h);
            log!("  background alone: rendered {done}, hash 0x{h:08x}");
        }
        let (d1, us1, list) = draw(e, vram, w, 24);
        // G4: the start table's words Dell's way, read back from the core
        // after the render (that they were written, and held).
        if let Way::Direct(f) = w
            && !f.regs.is_empty()
        {
            let _ = write!(RunLog, "  read back after the render:");
            for &(off, _) in f.regs {
                let _ = write!(RunLog, " 0x{off:x} 0x{:08x}", e.regs.read32(MBX + off));
            }
            log!("");
        }
        // G5: Dell's MBX words read back from the core after the render
        // (that they reached it; 0x6D0 and the TA's words read safely
        // under Windows Mobile, rp-wmprobe v3).
        // G6-G8: the layout's addresses as the core holds them.
        if let Way::G6(_) | Way::Frame(_) | Way::G8(_) = w {
            let _ = write!(RunLog, "  layout read back after the render:");
            for off in [
                0x608, 0x60C, 0x650, 0x65C, 0x660, 0x824, 0x828, 0x82C, 0x83C, 0x840, 0x844,
            ] {
                let _ = write!(RunLog, " 0x{off:x} 0x{:08x}", e.regs.read32(MBX + off));
            }
            log!("");
        }
        let dell_mask = match w {
            Way::Dell(m) => Some(m),
            Way::G6(k) if k >= 2 => Some(super::DELL_ALL),
            _ => None,
        };
        if let Some(m) = dell_mask {
            let _ = write!(RunLog, "  read back after the render:");
            for (k, (_, it)) in super::DELL_ITEMS.iter().enumerate() {
                if m >> k & 1 == 1
                    && let DellItem::Mbx(off, _) = *it
                {
                    let _ = write!(RunLog, " 0x{off:x} 0x{:08x}", e.regs.read32(MBX + off));
                }
            }
            log!("");
        }
        keep_reference(vram);
        // G7: the replay at Windows Mobile's clock against Dell's own
        // picture (his divide-1 render, right, hashed by g7-frame).
        let replay_way = match w {
            Way::Frame(_) => true,
            Way::G10(k) => super::G10_WAYS[k as usize % super::G10_WAYS.len()].0,
            Way::G11(k) => super::G11_WAYS[k as usize % super::G11_WAYS.len()].0,
            Way::G12(k) => super::G12_WAYS[k as usize % super::G12_WAYS.len()].0,
            Way::G13(k) => super::G13_WAYS[k as usize % super::G13_WAYS.len()].0,
            Way::G8(k) => super::G8_WAYS[k as usize % super::G8_WAYS.len()].0,
            _ => false,
        };
        if let (true, Some(f)) = (replay_way, frame()) {
            let h = picture_hash(vram, REF);
            if f.picture_words == 0 {
                log!("  Dell's own picture: none in this frame (hash 0x{h:08x})");
            } else {
                log!(
                    "  against Dell's own picture: hash 0x{h:08x}, his 0x{:08x} ({} words, ours {}): {}",
                    f.picture_hash,
                    f.picture_words,
                    target_words(),
                    if h == f.picture_hash && f.picture_words as usize == target_words() {
                        "the same"
                    } else {
                        "DIFFERENT"
                    }
                );
            }
        }
        // The reference's hash: ways that must draw the same picture (G2's
        // forms of one quad over one background) are held to each other,
        // not only each to itself.
        let ref_hash = picture_hash(vram, REF);
        log!("  reference hash 0x{ref_hash:08x}");
        // The reference must hold the quad (G4, 2026-10-23: G2'"'"'s colourless
        // clear drawn alone first left the next renders all white, and white
        // against white read as "20 of 20 right"): not its background alone,
        // and the quad'"'"'s black and white corners told apart.
        let corner = |x: u32, y: u32| {
            load32_volatile(vram, REF + 4 * ((y * W + x) / per_word()) as usize).unwrap_or(0)
        };
        let quad_shown = bg_hash != Some(ref_hash) && corner(2, 2) != corner(477, 2);
        // Judged only on a picture that rendered: a reference that never
        // drew (the TA silent) is the no-render case, not this one (G6's
        // TA way in the emulator, 2026-10-23).
        if d1 && !quad_shown {
            log!(
                "  the reference holds no quad (its background alone, or the corners 0x{:08x} and 0x{:08x} alike): not a reference",
                corner(2, 2),
                corner(477, 2)
            );
        }
        let (d2, us2, _) = draw(e, vram, w, 0);
        let again = compare(vram, REF);
        r.repeatable = d1 && d2 && again.wrong == 0 && quad_shown;
        r.us_wm = us1.min(us2);
        log!(
            "  reference at Windows Mobile's clock: memory manager ready {ready}, rendered {d1} then {d2}, the second {} words off the first; {} us",
            again.wrong,
            r.us_wm
        );
        if !r.repeatable {
            // What differs (test G's way 1 differed in every word): five
            // places, the first render's word then the second's.
            let _ = write!(RunLog, "    samples (first, second):");
            for (x, y) in [(2, 2), (240, 320), (477, 2), (2, 637), (477, 637)] {
                let at = 4 * ((y * W + x) / per_word()) as usize;
                let _ = write!(
                    RunLog,
                    " ({x}, {y}) 0x{:08x} 0x{:08x};",
                    load32_volatile(vram, REF + at).unwrap_or(0),
                    load32_volatile(vram, lay().target + at).unwrap_or(0)
                );
            }
            log!("");
            log!("  no repeatable reference: nothing at divide 1");
            return (r, list);
        }
        for (k, &(pll, mhz)) in settings.iter().take(SETTINGS).enumerate() {
            let s = &mut r.at[k];
            let mut line = [0u8; 32];
            let line = fmt(&mut line, format_args!("{mhz} MHZ / 1: SETTING"));
            say(vram, &[G_TITLE, head, name, line]);
            let dell = match w {
                Way::Dell(m) => m,
                Way::G6(k) if k >= 2 => super::DELL_ALL,
                Way::Frame(true) => super::DELL_ALL,
                Way::G11(k) if super::G11_WAYS[k as usize % super::G11_WAYS.len()].2 => {
                    super::DELL_ALL
                }
                _ => 0,
            };
            s.clocked = to_divide_1(
                &mut e.regs,
                Speed::ladder(pll),
                has(dell, DellItem::ClockOrder),
            );
            // G10: the per-render ways start each render at Windows
            // Mobile's clock and change it themselves.
            if let (true, Way::G10(k)) = (s.clocked, w)
                && super::G10_WAYS[k as usize % super::G10_WAYS.len()].1 != super::G10Mode::AsBefore
            {
                clock_back(&mut e.regs);
                G10_PLL.with(|p| *p = (pll, 0, 0));
            }
            if s.clocked && G11_CS2.with(|c| *c) {
                log!(
                    "    MSC1 0x{:08x} (CS2's half Dell's at divide 1)",
                    diag::msc1()
                );
            }
            if s.clocked && has(dell, DellItem::Nintpw) {
                let v = super::nintpw(Speed::ladder(pll).khz());
                e.regs.write32(super::NINTPW, v);
                log!(
                    "    NINTPW 0x{:x} by Dell's rule (read back 0x{:x})",
                    v,
                    e.regs.read32(super::NINTPW)
                );
            }
            clock_readback(&mut e.regs);
            if !s.clocked {
                log!(
                    "  the clock change was refused (Windows Mobile's clock kept): nothing at divide 1"
                );
                continue;
            }
            let ready = clean(e, vram);
            log!("  at {mhz} / 1: memory manager ready {ready}");
            s.screen = display_check(&mut e.regs, "after the change");
            s.us = u32::MAX;
            for n in 1..=G_DRAWS {
                rt::check_in();
                let mut line = [0u8; 32];
                let line = fmt(&mut line, format_args!("{mhz} MHZ / 1: {n} OF {G_DRAWS}"));
                say(vram, &[G_TITLE, head, name, line]);
                let (done, us, _) = draw(e, vram, w, 0);
                s.us = s.us.min(us);
                if !done {
                    s.unfinished += 1;
                    let _ = clean(e, vram);
                }
                let t = compare(vram, REF);
                if t.wrong == 0 {
                    s.right += 1;
                } else {
                    rt::delay_us(50_000);
                    if compare(vram, REF) != t {
                        s.unsettled += 1;
                    }
                }
                s.tally.merge(&t);
            }
            let (g10, changes, dark) = G10_PLL.with(|p| core::mem::replace(p, (0, 0, 0)));
            if g10 != 0 {
                log!(
                    "    {changes} clock changes around the renders, the display not sending after {dark}"
                );
                s.screen &= dark == 0;
            }
            s.screen &= display_check(&mut e.regs, "after the draws");
            clock_back(&mut e.regs);
            s.screen &= display_check(&mut e.regs, "back at Windows Mobile's clock");
            clock_readback(&mut e.regs);
            let _ = clean(e, vram);
            let (done, _, _) = draw(e, vram, w, 0);
            s.control = done && compare(vram, REF).wrong == 0;
            let t = &s.tally;
            let _ = write!(
                RunLog,
                "  at {mhz} / 1: {} of {G_DRAWS} right ({} unfinished, {} read differently 50 ms later); wrong words {} ({} one step, {} never written, worst step {})",
                s.right, s.unfinished, s.unsettled, t.wrong, t.one_step, t.unwritten, t.max_step
            );
            if let Some((i, want, got)) = t.first {
                let _ = write!(
                    RunLog,
                    ", first at ({}, {}) 0x{got:08x} for 0x{want:08x}",
                    i % W,
                    i / W
                );
            }
            if (TEST_G2 || TEST_G3 || TEST_G4) && t.wrong > 0 {
                let _ = write!(RunLog, "; on the picture's edge {}", t.edge);
                if let Some((i, want, got)) = t.first_inner {
                    let _ = write!(
                        RunLog,
                        ", first inside at ({}, {}) 0x{got:08x} for 0x{want:08x}",
                        i % W,
                        i / W
                    );
                }
            }
            log!(
                "; fastest {} us (Windows Mobile's {} us); control at Windows Mobile's clock after: {}; the screen on throughout: {}",
                s.us,
                r.us_wm,
                if s.control { "right" } else { "WRONG" },
                if s.screen {
                    "yes"
                } else {
                    "NO (a screen-off result)"
                }
            );
        }
        (r, list)
    }

    /// `args` formatted into `buf` (cut at its end).
    fn fmt<'a>(buf: &'a mut [u8], args: core::fmt::Arguments) -> &'a str {
        struct Cut<'b>(&'b mut [u8], usize);
        impl Write for Cut<'_> {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                for &b in s.as_bytes() {
                    if self.1 < self.0.len() {
                        self.0[self.1] = b;
                        self.1 += 1;
                    }
                }
                Ok(())
            }
        }
        let mut c = Cut(buf, 0);
        let _ = c.write_fmt(args);
        let n = c.1;
        core::str::from_utf8(&buf[..n]).unwrap_or("")
    }

    /// Test G: the three ways, then the strip of four; each against its
    /// own reference. The summary on the label.
    #[inline(never)]
    fn test_g(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G: Gouraud at 74.75 MHz / 1, through the TA against our objects (TSP 0x{TSP_SMOOTH:08x}, GC2's ARGB8888 quad)"
        );
        rp_image::on_crash(put_back);
        let at = &super::G3_PLLS[..1];
        let (ta, list) = way(e, vram, Way::Ta, at, 2, 5);
        let (ours, _) = way(e, vram, Way::Ours, at, 3, 5);
        let flags = gouraud_word(vram, &list);
        let mixed = match flags {
            Some(p) => {
                let (f, all) = adopt_flags(STRIP, p & super::FLAGS);
                log!(
                    "G: the TA's Gouraud list word 0x{p:08x} (layout {}); way 3 takes {} (0x{f:08x})",
                    (p & super::LAYOUT) >> 21,
                    if all {
                        "all of its flags"
                    } else {
                        "its layout only (its triangle mask differs from ours)"
                    }
                );
                Some(way(e, vram, Way::OursTaFlags(f), at, 4, 5).0)
            }
            None => {
                log!("G way 3: skipped (the TA wrote no Gouraud object in the first tile)");
                None
            }
        };
        let (strip4, _) = way(e, vram, Way::TaStrip4, at, 5, 5);
        let count = |r: &WayResult| r.right(0).unwrap_or(0);
        log!(
            "G: right at 74.75 / 1 of {G_DRAWS}: through the TA {}, ours {}, ours with the TA's flags {}, the TA's strip of four {}",
            count(&ta),
            count(&ours),
            mixed.as_ref().map_or(0, count),
            count(&strip4)
        );
        let mut l = [[0u8; 32]; 4];
        let [l0, l1, l2, l3] = &mut l;
        let a = fmt(l0, format_args!("TA: {} OF {G_DRAWS} RIGHT", count(&ta)));
        let b = fmt(l1, format_args!("OURS: {} OF {G_DRAWS}", count(&ours)));
        let c = fmt(
            l2,
            format_args!("TA FLAGS: {} OF {G_DRAWS}", mixed.as_ref().map_or(0, count)),
        );
        let d = fmt(
            l3,
            format_args!("STRIP OF 4: {} OF {G_DRAWS}", count(&strip4)),
        );
        say(
            vram,
            &["RP-GLESFEAT TEST G", "DONE: PRESS A BUTTON", a, b, c, d],
        );
    }

    /// Test G2's ways, in order: our form (G's way 2, the control), each
    /// of the TA's four choices alone, all four together, then the TA
    /// itself (last: a TA hang isn't recovered within a run, C53).
    const G2_WAYS: [Way; 8] = {
        const OURS: Form = Form {
            name: "ours (G's way 2)",
            quad: false,
            z1: false,
            bg: Bg::Giant,
            tiled: false,
            regs: &[],
        };
        [
            Way::Direct(OURS),
            Way::Direct(Form {
                name: "one 4-corner object",
                quad: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "z 1.0",
                z1: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "the clear underneath",
                bg: Bg::Clear,
                ..OURS
            }),
            Way::Direct(Form {
                name: "per-tile lists",
                tiled: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "all four: the TA's form",
                quad: true,
                z1: true,
                bg: Bg::Clear,
                tiled: true,
                regs: &[],
            }),
            Way::TaStrip4,
            Way::Ta,
        ]
    };

    /// Test G2: each of [`G2_WAYS`] against its own reference; the summary
    /// on the label.
    #[inline(never)]
    fn test_g2(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G2: which of the TA's choices makes Gouraud right at 74.75 MHz / 1 (TSP 0x{TSP_SMOOTH:08x}, GC2's ARGB8888 quad)"
        );
        rp_image::on_crash(put_back);
        let steps = G2_WAYS.len() as u32 + 1;
        let mut right = [None; G2_WAYS.len()];
        for (k, &w) in G2_WAYS.iter().enumerate() {
            let (r, _) = way(e, vram, w, &super::G3_PLLS[..1], k as u32 + 2, steps);
            right[k] = r.right(0);
        }
        let _ = write!(RunLog, "G2: right at 74.75 / 1 of {G_DRAWS}:");
        for (k, w) in G2_WAYS.iter().enumerate() {
            match right[k] {
                Some(n) => {
                    let _ = write!(RunLog, " {} {n};", w.name());
                }
                None => {
                    let _ = write!(RunLog, " {} not run;", w.name());
                }
            }
        }
        log!("");
        let mut l = [[0u8; 32]; 8];
        let mut lines: [&str; 10] = [""; 10];
        lines[0] = G_TITLE;
        lines[1] = "DONE: PRESS A BUTTON";
        for (k, (buf, w)) in l.iter_mut().zip(G2_WAYS.iter()).enumerate() {
            lines[2 + k] = match right[k] {
                Some(n) => fmt(buf, format_args!("{}: {n} OF {G_DRAWS}", k + 1)),
                None => fmt(buf, format_args!("{}: NOT RUN", k + 1)),
            };
            let _ = w;
        }
        say(vram, &lines);
    }

    /// Test G3's ways (the quad our two Gouraud triangles unless named):
    /// our background (G2's way 1, the control), Dell's colour clear by
    /// hand, Dell's whole sequence by hand, our background's traits one at a
    /// time against Dell's (its size, its depth), the other backgrounds a
    /// renderer might use (Gouraud, none), then Dell's sequence through the
    /// TA (last: a TA hang isn't recovered within a run, C53).
    const G3_WAYS: [Way; 8] = {
        const OURS: Form = Form {
            name: "our giant flat triangle",
            quad: false,
            z1: false,
            bg: Bg::Giant,
            tiled: false,
            regs: &[],
        };
        [
            Way::Direct(OURS),
            Way::Direct(Form {
                name: "Dell's clear (a flat quad)",
                bg: Bg::DellQuad,
                ..OURS
            }),
            Way::Direct(Form {
                name: "Dell's clear and 4-corner quad",
                bg: Bg::DellQuad,
                quad: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "flat, screen-sized triangles",
                bg: Bg::ScreenTris,
                ..OURS
            }),
            Way::Direct(Form {
                name: "our giant triangle at z 1.0",
                bg: Bg::GiantZ1,
                ..OURS
            }),
            Way::Direct(Form {
                name: "a Gouraud giant triangle",
                bg: Bg::Gouraud,
                ..OURS
            }),
            Way::Direct(Form {
                name: "no background",
                bg: Bg::Nothing,
                ..OURS
            }),
            Way::TaDell,
        ]
    };

    /// Test G4's ways: our form (the control), G2's colourless clear (the one
    /// form seen right at divide 1), Dell's render table whole (with our
    /// 32-bit target, then with Dell's 16-bit one: Dell's set-up exactly),
    /// then each of Dell's words alone.
    const G4_WAYS: [Way; 10] = {
        const OURS: Form = Form {
            name: "ours",
            quad: false,
            z1: false,
            bg: Bg::Giant,
            tiled: false,
            regs: &[],
        };
        const D: [(usize, u32); 7] = super::DELL_TABLE;
        const ALL16: [(usize, u32); 9] = [
            D[0],
            D[1],
            D[2],
            D[3],
            D[4],
            D[5],
            D[6],
            super::DELL_TARGET16[0],
            super::DELL_TARGET16[1],
        ];
        const fn one(name: &'static str, regs: &'static [(usize, u32)]) -> Way {
            Way::Direct(Form { name, regs, ..OURS })
        }
        [
            Way::Direct(OURS),
            Way::Direct(Form {
                name: "G2's colourless clear",
                bg: Bg::Clear,
                ..OURS
            }),
            one("Dell's table, 32-bit", &D),
            one("Dell's table and RGB565", &ALL16),
            one("Dell's 0x610 alone", &[D[0]]),
            one("Dell's 0x614, 0x618 alone", &[D[1], D[2]]),
            one("Dell's 0x61c alone", &[D[3]]),
            one("Dell's 0x648 alone", &[D[4]]),
            one("Dell's 0x6d8 alone", &[D[5]]),
            one("Dell's 0x6dc alone", &[D[6]]),
        ]
    };

    /// The most ways [`test_settings`] takes.
    const MAX_WAYS: usize = 10;

    /// Test G3 or G4: each of `ways` at each of [`super::G3_PLLS`], against
    /// its own reference; the summary in the log (each screen-off result
    /// marked) and on the label.
    #[inline(never)]
    fn test_settings(e: &mut Engine<Mmio>, vram: &mut [u8], ways: &[Way]) {
        rp_image::on_crash(put_back);
        let ways = &ways[..ways.len().min(MAX_WAYS)];
        let before = WAYS_BEFORE.with(|b| *b);
        let steps = ways.len() as u32 + 1 + before;
        let mut right = [[None; SETTINGS]; MAX_WAYS];
        let mut dark = [[false; SETTINGS]; MAX_WAYS];
        for (k, &w) in ways.iter().enumerate() {
            let (r, _) = way(e, vram, w, &super::G3_PLLS, k as u32 + 2 + before, steps);
            for s in 0..SETTINGS {
                right[k][s] = r.right(s);
                dark[k][s] = r.right(s).is_some() && !r.at[s].screen;
            }
        }
        for (s, (_, mhz)) in super::G3_PLLS.iter().enumerate() {
            let _ = write!(RunLog, "{G_NAME}: right at {mhz} / 1 of {G_DRAWS}:");
            for (k, w) in ways.iter().enumerate() {
                match right[k][s] {
                    Some(n) => {
                        let off = if dark[k][s] { " (screen off)" } else { "" };
                        let _ = write!(RunLog, " {} {n}{off};", w.name());
                    }
                    None => {
                        let _ = write!(RunLog, " {} not run;", w.name());
                    }
                }
            }
            log!("");
        }
        let mut l = [[0u8; 32]; MAX_WAYS];
        let mut lines: [&str; 5 + MAX_WAYS] = [""; 5 + MAX_WAYS];
        lines[0] = G_TITLE;
        lines[1] = "DONE: PRESS A BUTTON";
        lines[2] = "RIGHT OF 20 AT 74/99/133";
        lines[3] = "* = THE SCREEN WENT DARK";
        // G14: its first way's result (at Dell's own clock) above the rest.
        let mut first = [0u8; 32];
        let top = match FIRST_WAY.with(|f| *f) {
            Some(Some(n)) => {
                lines[4] = fmt(&mut first, format_args!("WAY 1: {n} AT DELL'S CLOCK"));
                5
            }
            Some(None) => {
                lines[4] = "WAY 1: NOT RUN";
                5
            }
            None => 4,
        };
        for (k, buf) in l.iter_mut().enumerate().take(ways.len()) {
            let n = |s: usize| right[k][s].map_or(-1, |v| v as i32);
            let d = |s: usize| if dark[k][s] { "*" } else { "" };
            lines[top + k] = fmt(
                buf,
                format_args!(
                    "WAY {}: {}{} {}{} {}{}",
                    k as u32 + 1 + before,
                    n(0),
                    d(0),
                    n(1),
                    d(1),
                    n(2),
                    d(2)
                ),
            );
        }
        say(vram, &lines[..top + ways.len()]);
    }

    /// Test G5's form: ours exactly (two Gouraud triangles over our giant
    /// flat background, z 0), every list word and table word ours unless
    /// a Dell item says otherwise.
    const G5_OURS: Form = Form {
        name: "ours",
        quad: false,
        z1: false,
        bg: Bg::Giant,
        tiled: false,
        regs: &[],
    };

    /// Where test G5 keeps the picture drawn on the chip as Dell's driver
    /// left it (after the TA's region array, before ours).
    const INHERIT: usize = 0x00B0_0000;
    const _: () = assert!(INHERIT >= TA_REGIONS + 0x1_0000);
    const _: () = assert!(INHERIT + STRIDE * H as usize <= REGIONS);

    /// Whether `mask` holds `item`.
    fn has(mask: u32, item: DellItem) -> bool {
        super::DELL_ITEMS
            .iter()
            .enumerate()
            .any(|(k, (_, it))| *it == item && mask >> k & 1 == 1)
    }

    /// `mask`'s MBX words for one render: the start table's own words into
    /// `over` (replaced there), the others written to the core now, just
    /// before it. The count put in `over`.
    fn dell_words(e: &mut Engine<Mmio>, mask: u32, over: &mut [(usize, u32); 16]) -> usize {
        let (t, len) = table();
        let mut n = 0;
        for (k, (_, it)) in super::DELL_ITEMS.iter().enumerate() {
            if mask >> k & 1 == 1
                && let DellItem::Mbx(off, v) = *it
            {
                if t[..len - 1].iter().any(|p| p.0 == off) {
                    if n < over.len() {
                        over[n] = (off, v);
                        n += 1;
                    }
                } else {
                    e.regs.write32(MBX + off, v);
                }
            }
        }
        n
    }

    /// `mask`'s items by name, into the log line being written.
    fn log_items(mask: u32) {
        for (k, (name, _)) in super::DELL_ITEMS.iter().enumerate() {
            if mask >> k & 1 == 1 {
                let _ = write!(RunLog, " [{name}]");
            }
        }
    }

    /// The MBX words [`state_diff`] reads: the reset, the enables, the
    /// render block and the tile accelerator's (all read safely under
    /// Windows Mobile on this X51v, `rp-wmprobe` v3).
    fn state_words() -> impl Iterator<Item = usize> {
        [0x080usize, 0x130]
            .into_iter()
            .chain((0x600..=0x6FC).step_by(4))
            .chain((0x800..=0x87C).step_by(4))
    }

    /// The chip's state against Dell's after his draws at 74.75 MHz / 1
    /// (`rp-wmprobe` v3): every word that differs logged, ours then his;
    /// the frame's buffer addresses and status words left out. The count
    /// that differ. Each block's line is started before its reads.
    fn state_diff(regs: &mut Mmio, when: &str) -> u32 {
        let mut n = 0;
        log!("  {when}: the chip against Dell's at 74.75 / 1 (system block, then the MBX core's)");
        let _ = write!(RunLog, "   ");
        let sys = super::DELL_SYS
            .iter()
            .copied()
            .chain([(super::NINTPW, super::nintpw(74_750))]);
        for (off, dell) in sys {
            let v = regs.read32(off);
            if v != dell {
                n += 1;
                let _ = write!(RunLog, " SYS 0x{off:x} 0x{v:x} (Dell 0x{dell:x});");
            }
        }
        log!("");
        let _ = write!(RunLog, "   ");
        for o in state_words() {
            if super::DELL_ADDRESSES.contains(&o) {
                continue;
            }
            let dell = super::DELL_MBX.iter().find(|p| p.0 == o).map_or(0, |p| p.1);
            let v = regs.read32(MBX + o);
            if v != dell {
                n += 1;
                let _ = write!(RunLog, " MBX 0x{o:x} 0x{v:x} (Dell 0x{dell:x});");
            }
        }
        log!("");
        log!("  {when}: {n} words differ from Dell's");
        n
    }

    /// Test G5, first when the image starts at the cores' divide 1
    /// (Windows Mobile booted with Dell's `CoreClock`, the loader started
    /// after): our quad drawn on the chip as Dell's driver left it, before
    /// anything of ours resets or rewrites it (only our memory manager set
    /// up: Dell's points into his memory). Kept at [`INHERIT`]; how many
    /// words two such renders differ in, if both drew.
    fn inherit(e: &mut Engine<Mmio>, vram: &mut [u8], clk0: u32) -> Option<u32> {
        say(vram, &[G_TITLE, "STEP 1 OF 6", "DELL'S CHIP AS FOUND"]);
        log!(
            "G5 inherited: the image started at the cores' divide 1 (MBXCLK 0x{clk0:x}): our quad on Dell's chip as found"
        );
        clock_readback(&mut e.regs);
        let _ = state_diff(&mut e.regs, "as found");
        clock::restore(&mut e.regs, with_3d(clk0));
        rt::delay_us(1_000);
        let ready = pm_init(e, vram);
        let (d1, us, _) = draw(e, vram, Way::Direct(G5_OURS), 24);
        for k in 0..(W * H) as usize {
            let v = load32_volatile(vram, TARGET + 4 * k).unwrap_or(0);
            let _ = store32_volatile(vram, INHERIT + 4 * k, v);
        }
        cpu::drain_write_buffer();
        let (d2, _, _) = draw(e, vram, Way::Direct(G5_OURS), 0);
        let again = compare(vram, INHERIT);
        log!(
            "G5 inherited: memory manager ready {ready}, rendered {d1} then {d2} ({us} us), the second {} words off the first, hash 0x{:08x}",
            again.wrong,
            picture_hash(vram, INHERIT)
        );
        (ready && d1 && d2).then_some(again.wrong)
    }

    /// Test G5: Dell's whole state copied under the loader, then halved.
    /// Way 1 ours (the control), way 2 ours with every item of
    /// [`super::DELL_ITEMS`], at 74.75, 99.67 and 133.25 MHz / 1; the chip
    /// against Dell's at 74.75 / 1 after ours drew; then, at the first
    /// setting where ours is wrong and Dell's whole state right in every
    /// render, the items halved (checks of one setting each, at most
    /// [`super::NARROW_CHECKS`]) to the fewest still right, and those
    /// verified at all three settings.
    #[inline(never)]
    fn test_g5(e: &mut Engine<Mmio>, vram: &mut [u8], inherited: Option<u32>) {
        log!(
            "G5: Dell's chip state (rp-wmprobe v3, after his draws at 74.75 / 1) copied under the loader, at 74.75, 99.67 and 133.25 MHz, then halved to what matters (TSP 0x{TSP_SMOOTH:08x}, GC2's quad over our flat background)"
        );
        rp_image::on_crash(put_back);
        let enable0 = e.regs.read32(ENABLE);
        let culprit = G5_CULPRIT.and_then(|c| c.parse::<u32>().ok());
        if let Some(c) = culprit {
            log!("G5: the emulator's check: a check is right only with item {c} in its mask");
        }
        let plls = &super::G3_PLLS;
        let (ours, _) = way(e, vram, Way::Direct(G5_OURS), plls, 2, 6);
        // The inherited picture against ours's reference (REF holds it).
        let inherited_wrong = inherited.map(|again| {
            for k in 0..(W * H) as usize {
                let v = load32_volatile(vram, INHERIT + 4 * k).unwrap_or(0);
                let _ = store32_volatile(vram, TARGET + 4 * k, v);
            }
            cpu::drain_write_buffer();
            let t = compare(vram, REF);
            let _ = write!(
                RunLog,
                "G5 inherited against our reference at Windows Mobile's clock: {} words differ ({} one step, worst step {})",
                t.wrong,
                t.one_step,
                t.max_step
            );
            if let Some((i, want, got)) = t.first_inner {
                let _ = write!(
                    RunLog,
                    ", first inside at ({}, {}) 0x{got:08x} for 0x{want:08x}",
                    i % W,
                    i / W
                );
            }
            log!("; its two renders {again} words apart");
            t.wrong
        });
        // The chip against Dell's where he drew right: ours at 74.75 / 1
        // after one render.
        say(vram, &[G_TITLE, "STEP 2 OF 6", "OURS AGAINST DELL'S STATE"]);
        log!("G5 state: ours at {} / 1 against Dell's", plls[0].1);
        if to_divide_1(&mut e.regs, Speed::ladder(plls[0].0), false) {
            let _ = display_check(&mut e.regs, "after the change");
            let _ = clean(e, vram);
            let _ = draw(e, vram, Way::Direct(G5_OURS), 0);
            let _ = state_diff(&mut e.regs, "ours at 74.75 / 1 after a render");
            clock_back(&mut e.regs);
            let _ = display_check(&mut e.regs, "back at Windows Mobile's clock");
        } else {
            log!("  the clock change was refused: the state not compared at divide 1");
        }
        let (full, _) = way(e, vram, Way::Dell(super::DELL_ALL), plls, 3, 6);
        let right = |r: &WayResult, k: usize| r.right(k) == Some(G_DRAWS);
        let at =
            (0..SETTINGS).find(|&k| (!right(&ours, k) || culprit.is_some()) && right(&full, k));
        let mut found = None;
        match at {
            Some(k) => {
                log!(
                    "G5: ours wrong and Dell's whole state right at {} / 1: halving there",
                    plls[k].1
                );
                let one = [plls[k]];
                let (m, checks) = super::narrow(super::DELL_ALL, super::NARROW_CHECKS, |m| {
                    let (r, _) = way(e, vram, Way::Dell(m), &one, 4, 6);
                    let ok = r.right(0) == Some(G_DRAWS) && culprit.is_none_or(|c| m >> c & 1 == 1);
                    let _ = write!(
                        RunLog,
                        "G5 check 0x{m:05x} {}:",
                        if ok { "right" } else { "wrong" }
                    );
                    log_items(m);
                    log!("");
                    ok
                });
                let _ = write!(RunLog, "G5: found in {checks} checks, mask 0x{m:05x}:");
                log_items(m);
                log!("");
                found = Some(m);
            }
            None if (0..SETTINGS).all(|k| right(&ours, k)) => {
                log!("G5: ours is right at every setting here: nothing to halve");
            }
            None => {
                log!(
                    "G5: Dell's whole state copied doesn't draw right where ours is wrong: what differs isn't in the state copied (an order, or a word not readable)"
                );
            }
        }
        let verified = found.map(|m| way(e, vram, Way::Dell(m), plls, 5, 6).0);
        e.regs.write32(ENABLE, enable0);
        // The summary.
        let row = |r: &WayResult, k: usize| r.right(k).map_or(-1, |v| v as i32);
        for (k, (_, mhz)) in plls.iter().enumerate() {
            let _ = write!(
                RunLog,
                "G5: right at {mhz} / 1 of {G_DRAWS}: ours {}; Dell's whole state {}",
                row(&ours, k),
                row(&full, k)
            );
            if let Some(v) = &verified {
                let _ = write!(RunLog, "; the items found {}", row(v, k));
            }
            log!("");
        }
        let [l0, l1, l2, l3] = &mut [[0u8; 32]; 4];
        let lines = [
            G_TITLE,
            "DONE: PRESS A BUTTON",
            "RIGHT OF 20 AT 74/99/133",
            fmt(
                l0,
                format_args!(
                    "OURS: {} {} {}",
                    row(&ours, 0),
                    row(&ours, 1),
                    row(&ours, 2)
                ),
            ),
            fmt(
                l1,
                format_args!(
                    "DELL'S: {} {} {}",
                    row(&full, 0),
                    row(&full, 1),
                    row(&full, 2)
                ),
            ),
            match (&verified, found) {
                (Some(v), Some(m)) => fmt(
                    l2,
                    format_args!("FOUND {m:05X}: {} {} {}", row(v, 0), row(v, 1), row(v, 2)),
                ),
                _ => "NOTHING HALVED",
            },
            match inherited_wrong {
                Some(n) => fmt(l3, format_args!("ON DELL'S CHIP: {n} WRONG")),
                None => "NOT STARTED ON DELL'S CHIP",
            },
        ];
        say(vram, &lines);
    }

    /// Test G6's ways (each layout of [`super::G6_LAYOUTS`]).
    const G6_WAYS: [Way; 4] = [Way::G6(0), Way::G6(1), Way::G6(2), Way::G6(3)];

    /// Test G6: where things sit in local memory, the parameter memory's
    /// set-up and the render's flow, Dell's against ours (the leads G5 left).
    fn test_g6(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G6: Dell's places in local memory, his parameter memory and his flow, against ours, at 74.75, 99.67 and 133.25 MHz (TSP 0x{TSP_SMOOTH:08x}, GC2's quad over our flat background; Dell's places read by rp-wmprobe v3)"
        );
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G6_WAYS);
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
    }

    /// Test G7's replay of Dell's frame: its pages written into local
    /// memory, its registers (the parameter memory manager's aside, set up
    /// by [`clean`] from the frame's layout), then the 3D stage started.
    /// The target is blanked first by [`draw`]. Whether it finished.
    #[inline(never)]
    fn replay(e: &mut Engine<Mmio>, vram: &mut [u8]) -> bool {
        let Some(f) = frame() else {
            log!("  no frame: nothing replayed");
            return false;
        };
        for (addr, data) in f.segs() {
            for (k, c) in data.chunks_exact(4).enumerate() {
                let _ = store32_volatile(
                    vram,
                    addr + 4 * k,
                    u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                );
            }
        }
        cpu::drain_write_buffer();
        let mut pairs = [(0usize, 0u32); 160];
        let mut n = 0;
        for (off, v) in f.regs() {
            if super::PM_BLOCK.contains(&off) || n + 1 >= pairs.len() {
                continue;
            }
            pairs[n] = (off, v);
            n += 1;
        }
        pairs[n] = (m3::START, 1);
        let done = run(e, &pairs[..=n], RENDER_DONE, 200_000);
        if !done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("  (timed out: core reset, memory manager ready again: {ok})");
        }
        done
    }

    /// Test G7's ways: ours, Dell's frame replayed, the same with his
    /// NINTPW rule and clock order.
    const G7_WAYS: [Way; 3] = [Way::G6(0), Way::Frame(false), Way::Frame(true)];

    /// The image's frame (test G7's, Dell's own) and its layout, if it
    /// parses, has its layout's registers and lies over nothing of the
    /// image's own; logged either way. Every test that replays it asks
    /// first: the replay writes its pages over local memory.
    fn usable_frame() -> Option<super::Layout> {
        let Some(f) = frame() else {
            log!(
                "{G_NAME}: the frame doesn't parse ({:?}): nothing run",
                super::frame::parse(FRAME_FILE).err()
            );
            return None;
        };
        let Some(l) = super::frame_layout(&f) else {
            log!("{G_NAME}: the frame lacks its layout's registers: nothing run");
            return None;
        };
        let pages: usize = f.segs().map(|s| s.1.len()).sum();
        log!(
            "{G_NAME}: the frame: {} registers, {} bytes of local memory in {} segments; target 0x{:x} ({}), parameters 0x{:x} ({} KiB, table 0x{:x}), region array 0x{:x}",
            f.regs().count(),
            pages,
            f.segs().count(),
            l.target,
            if l.rgb565 { "RGB565" } else { "ARGB8888" },
            l.pm,
            l.pm_size >> 10,
            l.pm_table,
            l.regions
        );
        // Nothing of the image's own may be under the frame (the label is
        // scanned out, the reference kept, the fence the 2D engine's).
        let own = [
            (LABEL, LABEL_STRIDE * H as usize, "the label"),
            (REF, STRIDE * H as usize, "the reference"),
            (FENCE, 0x1000, "the fence"),
        ];
        let spans = l.spans(W, H, COLS * ROWS);
        for (from, bytes, what) in own {
            let hit = f.touches(from, bytes)
                || spans[..4]
                    .iter()
                    .any(|s| s.0 < from + bytes && from < s.0 + s.1);
            if hit {
                log!("{G_NAME}: the frame lies over {what} (0x{from:x}): nothing run");
                return None;
            }
        }
        Some(l)
    }

    /// Test G7: Dell's own divide-1 frame replayed under the loader.
    fn test_g7(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G7: Dell's own frame (his driver's region array, lists and objects as he left them after his divide-1 draws, rp-wmprobe v4) replayed word for word, at 74.75, 99.67 and 133.25 MHz, against ours"
        );
        if usable_frame().is_none() {
            return;
        }
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G7_WAYS);
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
    }

    /// Test G8's ways.
    const G8_WAYS: [Way; 5] = [Way::G8(0), Way::G8(1), Way::G8(2), Way::G8(3), Way::G8(4)];

    /// Test G8: the render waited for without touching the chip, or once a
    /// millisecond, ours and Dell's frame (G7's lead).
    fn test_g8(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G8: the render waited for without reading the chip (one read after {} ms) and with a read once a millisecond, ours and Dell's own frame, at 74.75, 99.67 and 133.25 MHz (render times include the wait)",
            super::QUIET_US / 1000
        );
        if usable_frame().is_none() {
            log!("G8: no frame, or one without its layout: nothing run");
            return;
        }
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G8_WAYS);
        WAIT.with(|v| *v = super::Wait::Tight);
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
    }

    /// Every word of the MBX core's block and of the system block's
    /// [`super::SYS_WORDS`], then the PXA27x's registers
    /// of `diag`'s list: `rp-wmprobe` v5's lines exactly, for the diff.
    /// Each line is written before the next read.
    fn full(regs: &mut Mmio, tag: &str) {
        log!("full {tag}: begins (MBX, SYS, PXA)");
        for off in (0..0x1000).step_by(4) {
            log!(
                "full {tag}: MBX 0x{off:03x} 0x{:08x}",
                regs.read32(MBX + off)
            );
        }
        for &off in &super::SYS_WORDS {
            log!("full {tag}: SYS 0x{off:04x} 0x{:08x}", regs.read32(off));
        }
        let mut snap = [0u32; diag::COUNT];
        diag::snapshot(&mut snap);
        let addrs = diag::GROUPS.iter().flat_map(|g| g.1.iter().map(|r| r.addr));
        for (addr, v) in addrs.zip(snap) {
            log!("full {tag}: PXA 0x{addr:08x} 0x{v:08x}");
        }
        log!("full {tag}: ends");
    }

    /// Test G9: every register under the loader after our render, at
    /// Windows Mobile's clock and at 74.75 / 1 (where it draws wrong), for
    /// the diff with `rp-wmprobe` v5's after Dell's draws (right).
    fn test_g9(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G9: every register of the 2700G's two blocks and the PXA27x's under the loader after our render, at Windows Mobile's clock and at 74.75 / 1"
        );
        rp_image::on_crash(put_back);
        say(
            vram,
            &[G_TITLE, "STEP 2 OF 3", "WM CLOCK: READING EVERY REGISTER"],
        );
        let ready = clean(e, vram);
        let (d, _, _) = draw(e, vram, Way::Direct(G5_OURS), 0);
        keep_reference(vram);
        log!("G9 at Windows Mobile's clock: memory manager ready {ready}, rendered {d}");
        full(&mut e.regs, "loader div2");
        say(
            vram,
            &[
                G_TITLE,
                "STEP 3 OF 3",
                "74.75 MHZ / 1: READING EVERY REGISTER",
            ],
        );
        if to_divide_1(&mut e.regs, Speed::ladder(super::G3_PLLS[0].0), false) {
            let screen = display_check(&mut e.regs, "after the change");
            let ready = clean(e, vram);
            let (d, _, _) = draw(e, vram, Way::Direct(G5_OURS), 0);
            let t = compare(vram, REF);
            log!(
                "G9 at 74.75 / 1: memory manager ready {ready}, rendered {d}, {} words off the reference (the screen on: {screen})",
                t.wrong
            );
            full(&mut e.regs, "loader div1");
            clock_back(&mut e.regs);
            let _ = display_check(&mut e.regs, "back at Windows Mobile's clock");
        } else {
            log!("G9: the clock change was refused: no divide-1 reading");
        }
        say(
            vram,
            &[G_TITLE, "DONE: PRESS A BUTTON", "EVERY REGISTER READ"],
        );
    }

    /// G10: to the divide-1 setting `pll` (or back to Windows Mobile's with
    /// `None`) around one render, the display checked after each change
    /// without a log line (ADR-0035 rule 8; the count is logged per
    /// setting): the label plane set again if it isn't sending.
    fn g10_change(e: &mut Engine<Mmio>, pll: Option<u32>) {
        match pll {
            Some(p) => {
                let _ = to_divide_1(&mut e.regs, Speed::ladder(p), false);
            }
            None => clock_back(&mut e.regs),
        }
        let _ = rp_drv_mbx_display::ack_events(&mut e.regs, GFX_UNDERRUN);
        rt::delay_us(FRAME_US);
        let a = e.regs.read32(DSIG);
        rt::delay_us(FRAME_US);
        let sending = a != 0 && e.regs.read32(DSIG) != 0;
        if !sending {
            let _ = show(
                &mut e.regs,
                Plane {
                    format: PixelFormat::Rgb565,
                    width: W as u16,
                    height: H as u16,
                    offset: LABEL,
                    stride: LABEL_STRIDE,
                },
            );
        }
        G10_PLL.with(|p| {
            p.1 += 1;
            p.2 += u32::from(!sending);
        });
    }

    /// Our scene (as [`G5_OURS`] draws it: our giant flat black background,
    /// GC2's two Gouraud triangles) written into local memory with the
    /// target blanked; the start table to render it.
    fn prepare_ours(vram: &mut [u8]) -> ([(usize, u32); 160], usize) {
        blank(vram);
        let bg = m3::triangle(
            ISP_ALWAYS,
            TSP_DELL,
            &[(0.0, 0.0), (2048.0, 0.0), (0.0, 2048.0)].map(|(x, y)| Vertex {
                x,
                y,
                z: 0.0,
                rhw: 1.0,
                argb: 0xFF00_0000,
            }),
        );
        let tri = |t: [usize; 3]| {
            m3::triangle(
                ISP_ALWAYS,
                TSP_SMOOTH,
                &t.map(|k| Vertex {
                    x: QUAD[k].0,
                    y: QUAD[k].1,
                    z: 0.0,
                    rhw: 1.0,
                    argb: QUAD[k].2,
                }),
            )
        };
        let (a, b) = (tri(TRIS[0]), tri(TRIS[1]));
        let params = lay().pm;
        let mut at = params + 64;
        let mut list = [m3::END; 4];
        for (k, w) in [&bg[..], &a[..], &b[..]].into_iter().enumerate() {
            store(vram, at, w);
            list[k] = STRIP | ((at - params) / 4) as u32;
            at += 4 * w.len();
        }
        store(vram, params, &list);
        regions(vram);
        let (t, len) = table();
        let mut pairs = [(0usize, 0u32); 160];
        pairs[..len].copy_from_slice(&t[..len]);
        (pairs, len)
    }

    /// Dell's frame (G7's) written into local memory with the target
    /// blanked; his registers and the start to render it.
    fn prepare_frame(vram: &mut [u8]) -> ([(usize, u32); 160], usize) {
        blank(vram);
        let mut pairs = [(0usize, 0u32); 160];
        let Some(f) = frame() else {
            return (pairs, 0);
        };
        for (addr, data) in f.segs() {
            for (k, c) in data.chunks_exact(4).enumerate() {
                let _ = store32_volatile(
                    vram,
                    addr + 4 * k,
                    u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                );
            }
        }
        cpu::drain_write_buffer();
        let mut n = 0;
        for (off, v) in f.regs() {
            if super::PM_BLOCK.contains(&off) || n + 1 >= pairs.len() {
                continue;
            }
            pairs[n] = (off, v);
            n += 1;
        }
        pairs[n] = (m3::START, 1);
        (pairs, n + 1)
    }

    /// The render started by `pairs` (the start last), waited for; a timeout
    /// resets the core.
    fn start(e: &mut Engine<Mmio>, vram: &mut [u8], pairs: &[(usize, u32)]) -> bool {
        if pairs.is_empty() {
            return false;
        }
        let done = run(e, pairs, RENDER_DONE, 200_000);
        if !done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("  (timed out: core reset, memory manager ready again: {ok})");
        }
        done
    }

    /// Test G10's ways.
    const G10_WAYS: [Way; 4] = [Way::G10(0), Way::G10(1), Way::G10(2), Way::G10(3)];

    /// Test G10: the render alone at divide 1, the CPU's writes and reads of
    /// the chip's memory at Windows Mobile's clock (G9's lead).
    fn test_g10(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G10: the CPU's writes and reads of the 2700G's memory moved to Windows Mobile's clock, the render alone at divide 1, ours and Dell's frame, at 74.75, 99.67 and 133.25 MHz (render times include the clock changes)"
        );
        if usable_frame().is_none() {
            log!("G10: no frame, or one without its layout: nothing run");
            return;
        }
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G10_WAYS);
        G10_PLL.with(|p| *p = (0, 0, 0));
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
    }

    /// Test G11's ways.
    const G11_WAYS: [Way; 4] = [Way::G11(0), Way::G11(1), Way::G11(2), Way::G11(3)];

    /// Test G11: CS2's timing as Dell's driver sets it at divide 1 (G10's
    /// lead), ours and Dell's frame.
    fn test_g11(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let found = diag::msc1();
        log!(
            "G11: chip select 2's timing (the 2700G's second interface, Dell's SRAMChipSelect 2) as Dell's driver sets it at divide 1 (0x{:04x}), ours and Dell's frame, at 74.75, 99.67 and 133.25 MHz; MSC1 as found 0x{found:08x}",
            super::DELL_CS2_DIV1
        );
        // Put back to Windows Mobile's after every change: only if that is
        // what was found (no silent change of the loader's own).
        if found & 0xFFFF != super::WM_CS2 {
            log!(
                "G11: CS2's half found 0x{:04x}, not Windows Mobile's 0x{:04x}: nothing run",
                found & 0xFFFF,
                super::WM_CS2
            );
            return;
        }
        if usable_frame().is_none() {
            log!("G11: no frame, or one without its layout: nothing run");
            return;
        }
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G11_WAYS);
        G11_CS2.with(|c| *c = false);
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
        log!("G11: MSC1 at the end 0x{:08x}", diag::msc1());
    }

    /// Test G12's ways.
    const G12_WAYS: [Way; 5] = [
        Way::G12(0),
        Way::G12(1),
        Way::G12(2),
        Way::G12(3),
        Way::G12(4),
    ];

    /// Test G12: the loader's environment while the chip renders made
    /// Windows Mobile's (W2's lead): the CPU silent, the loader-only clocks
    /// off.
    fn test_g12(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G12: the CPU kept off the bus while the chip renders ({} turns of a count down, then one status read) and the loader-only clocks (CKEN 0x{:08x} as found, 0x{:x} off) off during each render, Dell's frame and ours, at 74.75, 99.67 and 133.25 MHz",
            super::SILENT_SPINS,
            rt::clock_enables(),
            super::LOADER_ONLY_CLOCKS
        );
        if usable_frame().is_none() {
            log!("G12: no frame, or one without its layout: nothing run");
            return;
        }
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G12_WAYS);
        G12_CLOCKS.with(|c| *c = false);
        WAIT.with(|v| *v = super::Wait::Tight);
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
        log!("G12: CKEN at the end 0x{:08x}", rt::clock_enables());
    }

    /// The display's pixel clock set to `pll` (`DISPPLL`), its lock awaited
    /// (`PLLSTAT` bit 0, bounded) and the display checked; logged.
    fn set_display_pll(regs: &mut Mmio, pll: u32) {
        let was = regs.read32(rp_drv_mbx_display::reg::DISPPLL);
        if was != pll {
            regs.write32(rp_drv_mbx_display::reg::DISPPLL, pll);
            for _ in 0..10_000 {
                if regs.read32(PLLSTAT) & 1 != 0 {
                    break;
                }
            }
        }
        log!(
            "    DISPPLL 0x{was:x} -> 0x{:x} (PLLSTAT 0x{:x})",
            regs.read32(rp_drv_mbx_display::reg::DISPPLL),
            regs.read32(PLLSTAT)
        );
        let _ = display_check(regs, "after the display clock");
    }

    /// Test G13's ways.
    const G13_WAYS: [Way; 4] = [Way::G13(0), Way::G13(1), Way::G13(2), Way::G13(3)];

    /// Test G13: the display at Windows Mobile's pixel clock (G12's lead),
    /// Dell's frame and ours, against the loader's 60 Hz in the same run.
    fn test_g13(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let found = e.regs.read32(rp_drv_mbx_display::reg::DISPPLL);
        log!(
            "G13: the display at Windows Mobile's pixel clock (DISPPLL 0x{:x}) against the loader's (0x{:x}), Dell's frame and ours, at 74.75, 99.67 and 133.25 MHz; DISPPLL as found 0x{found:x}",
            rp_drv_mbx_display::pll::WM,
            rp_drv_mbx_display::pll::SIXTY
        );
        if usable_frame().is_none() {
            log!("G13: no frame, or one without its layout: nothing run");
            return;
        }
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G13_WAYS);
        set_display_pll(&mut e.regs, found);
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
    }

    /// Test G14's first way (W3's lead): Dell's frame on the chip as his
    /// driver set it at boot (Windows Mobile started with his `CoreClock`,
    /// the loader after), no clock of ours changed: only the 3D clock on at
    /// his divider (W2's step 1, harmless under Windows Mobile), our core
    /// reset and memory manager (W2's step 3, the same). Two renders, then
    /// [`G_DRAWS`] more, each held to his own picture (the frame's hash; to
    /// the first render when the frame carries none, as the emulator's).
    /// Every G7-G13 replay ran after our own clock change; G5's inherited
    /// way drew our quad here, never his frame.
    #[inline(never)]
    fn dells_clock(e: &mut Engine<Mmio>, vram: &mut [u8], clk0: u32) {
        WAYS_BEFORE.with(|b| *b = 1);
        FIRST_WAY.with(|f| *f = Some(None));
        let head = "STEP 2 OF 4";
        let name = "WAY 1: AT DELL'S CLOCK";
        say(vram, &[G_TITLE, head, name, "SETTING UP"]);
        log!(
            "{G_NAME} way 1 (Dell's frame at the clock his driver set at boot, no clock of ours changed):"
        );
        let pllstat = e.regs.read32(PLLSTAT);
        clock_readback(&mut e.regs);
        log!(
            "    as found: MBXCLK 0x{clk0:x} PLLSTAT 0x{pllstat:x} NINTPW 0x{:x} DISPPLL 0x{:x}; MSC1 0x{:08x} CKEN 0x{:08x}",
            e.regs.read32(super::NINTPW),
            e.regs.read32(rp_drv_mbx_display::reg::DISPPLL),
            diag::msc1(),
            rt::clock_enables()
        );
        if !super::dells_divide_1(clk0, pllstat) {
            if !G14_ANY_START {
                log!(
                    "  not at Dell's divide 1 (Windows Mobile booted without his CoreClock: rp-wmprobe's run 1 sets it, then a soft reset and the loader): not run"
                );
                return;
            }
            log!(
                "  not at Dell's divide 1: run anyway (RP_GLESFEAT_G14=any-start, the emulator's build)"
            );
        }
        let Some(l) = usable_frame() else {
            return;
        };
        let Some(f) = frame() else {
            return;
        };
        rp_image::on_crash(put_back);
        let _ = display_check(&mut e.regs, "as found");
        set_lay(l);
        let pll0 = e.regs.read32(clock::COREPLL);
        clock::restore(&mut e.regs, with_3d(clk0));
        rt::delay_us(1_000);
        let mut screen = display_check(&mut e.regs, "after the 3D clock on");
        let _ = state_diff(&mut e.regs, "at Dell's clock, before ours");
        let ready = clean(e, vram);
        let (d1, us1, _) = draw(e, vram, Way::Frame(false), 0);
        keep_reference(vram);
        let h = picture_hash(vram, REF);
        let his = f.picture_words != 0 && f.picture_words as usize == target_words();
        if his {
            log!(
                "  against Dell's own picture: hash 0x{h:08x}, his 0x{:08x}: {}",
                f.picture_hash,
                if h == f.picture_hash {
                    "the same"
                } else {
                    "DIFFERENT"
                }
            );
        } else {
            log!("  Dell's own picture: none in this frame (hash 0x{h:08x})");
        }
        log!("  reference hash 0x{h:08x}");
        let (d2, us2, _) = draw(e, vram, Way::Frame(false), 0);
        let again = compare(vram, REF);
        log!(
            "  reference at Dell's clock: memory manager ready {ready}, rendered {d1} then {d2}, the second {} words off the first; {} us",
            again.wrong,
            us1.min(us2)
        );
        let (mut right, mut unfinished, mut us) = (0u32, 0u32, u32::MAX);
        let mut tally = Tally::default();
        for n in 1..=G_DRAWS {
            rt::check_in();
            let mut line = [0u8; 32];
            let line = fmt(&mut line, format_args!("DELL'S CLOCK: {n} OF {G_DRAWS}"));
            say(vram, &[G_TITLE, head, name, line]);
            let (done, t_us, _) = draw(e, vram, Way::Frame(false), 0);
            us = us.min(t_us);
            if !done {
                unfinished += 1;
                let _ = clean(e, vram);
            }
            let t = compare(vram, REF);
            let ok = done
                && if his {
                    picture_hash(vram, lay().target) == f.picture_hash
                } else {
                    t.wrong == 0
                };
            right += u32::from(ok);
            tally.merge(&t);
        }
        screen &= display_check(&mut e.regs, "after the draws");
        // The way's premise, checked: the core PLL as found and the cores'
        // divider his, only the 3D clock turned on.
        let (pll, clk) = (e.regs.read32(clock::COREPLL), e.regs.read32(MBXCLK));
        log!(
            "  the clock as found throughout: {} (COREPLL 0x{pll0:x} -> 0x{pll:x}, MBXCLK 0x{clk0:x} -> 0x{clk:x})",
            pll == pll0 && clk == with_3d(clk0)
        );
        let t = &tally;
        log!(
            "  at Dell's clock: {right} of {G_DRAWS} right (held to {}); {unfinished} unfinished; against the first render {} words differ ({} one step, {} never written, worst step {}); fastest {us} us; the screen on throughout: {}",
            if his {
                "his own picture"
            } else {
                "the first render"
            },
            t.wrong,
            t.one_step,
            t.unwritten,
            t.max_step,
            if screen {
                "yes"
            } else {
                "NO (a screen-off result)"
            }
        );
        log!("{G_NAME}: right at Dell's own clock of {G_DRAWS}: Dell's frame {right}");
        FIRST_WAY.with(|f| *f = Some(Some(right)));
        set_lay(super::OURS_LAYOUT);
    }

    /// Test G14's ways after its first: ours and Dell's frame, each after
    /// our own clock change (G7's ways 1 and 2).
    const G14_WAYS: [Way; 2] = [Way::G6(0), Way::Frame(false)];

    /// Test G14: Dell's frame at the clock his driver set at boot
    /// ([`dells_clock`], run before Windows Mobile's clock is set), then
    /// ours and his frame after our own change to divide 1.
    fn test_g14(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G14: Dell's frame at the clock his driver set at boot (way 1, before anything of ours changed it), then ours and Dell's frame after our own clock change, at 74.75, 99.67 and 133.25 MHz"
        );
        if usable_frame().is_none() {
            return;
        }
        let enable0 = e.regs.read32(ENABLE);
        test_settings(e, vram, &G14_WAYS);
        set_lay(super::OURS_LAYOUT);
        e.regs.write32(ENABLE, enable0);
    }

    /// Test G3: the backgrounds.
    fn test_g3(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G3: which background lets Gouraud draw right at the cores' divide 1, at 74.75, 99.67 and 133.25 MHz (TSP 0x{TSP_SMOOTH:08x}, GC2's ARGB8888 quad; Dell's clear as libGLES_CL draws it)"
        );
        test_settings(e, vram, &G3_WAYS);
    }

    /// Test G4: Dell's render table.
    fn test_g4(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G4: Dell's render table at the cores' divide 1, whole and word by word, at 74.75, 99.67 and 133.25 MHz (TSP 0x{TSP_SMOOTH:08x}, GC2's quad; Dell's words read by rp-wmprobe at 74.75 / 1)"
        );
        test_settings(e, vram, &G4_WAYS);
    }

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(vram.len());
        let regs = display.into_inner();
        let mut e = Engine::new(regs, FENCE, rt::ticks, 3_250);
        // The label first: the screen says what runs from the first moment.
        let saved = show_label(&mut e.regs, vram);
        let title = if TEST_G {
            G_TITLE
        } else {
            "RP-GLESFEAT A D L M"
        };
        let first = if TEST_G14 {
            "STEP 1 OF 4"
        } else if TEST_G13 {
            "STEP 1 OF 5"
        } else if TEST_G12 {
            "STEP 1 OF 6"
        } else if TEST_G11 {
            "STEP 1 OF 5"
        } else if TEST_G10 {
            "STEP 1 OF 5"
        } else if TEST_G9 {
            "STEP 1 OF 3"
        } else if TEST_G8 {
            "STEP 1 OF 6"
        } else if TEST_G7 {
            "STEP 1 OF 4"
        } else if TEST_G6 {
            "STEP 1 OF 5"
        } else if TEST_G5 {
            "STEP 1 OF 6"
        } else if TEST_G4 {
            "STEP 1 OF 11"
        } else if TEST_G2 || TEST_G3 {
            "STEP 1 OF 9"
        } else {
            "STEP 1 OF 5"
        };
        say(vram, &[title, first, "SETTING UP THE 3D CORE"]);
        let clk0 = e.regs.read32(MBXCLK);
        if TEST_G {
            log!(
                "{MARKER}: test {G_NAME}, Gouraud at the cores' divide 1 (the give-up audit's item 1)"
            );
        } else {
            log!("{MARKER}: alpha test, depth 'less', 8-bit grey textures (ADR-0026)");
        }
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();
        let mut ready = false;
        // G5: started at the cores' divide 1 (Windows Mobile booted with
        // Dell's CoreClock): our quad on Dell's chip first, then Windows
        // Mobile's clock back for everything else.
        let mut inherited = None;
        if TEST_G5 && e.regs.read32(PLLSTAT) & 4 != 0 {
            // Divide 1 with a clock running, as Windows Mobile leaves it
            // under Dell's CoreClock (`0x1`: 2D on, rp-wmprobe run 2).
            if super::dells_divide_1(clk0, e.regs.read32(PLLSTAT)) {
                inherited = inherit(&mut e, vram, clk0);
            } else {
                log!(
                    "G5: started with MBXCLK 0x{clk0:x} (not divide 1 with a clock running): no inherited way (Windows Mobile booted without Dell's CoreClock)"
                );
            }
        }
        // G14: Dell's frame first on the chip as his driver set it at boot,
        // no clock of ours changed; then Windows Mobile's clock back as for
        // every G test, and our own clock change in the ways after.
        if TEST_G14 {
            dells_clock(&mut e, vram, clk0);
        }
        // Every G test: Windows Mobile's clock set here, not assumed. The
        // references are drawn at it; G6's first run (2026-10-23) found the
        // chip at Dell's divide 1 (Windows Mobile booted with his
        // CoreClock), drew them there, and none repeated.
        if TEST_G && e.regs.read32(PLLSTAT) & 4 != 0 {
            log!("started with MBXCLK 0x{clk0:x}: Windows Mobile's clock set");
            clock_back(&mut e.regs);
            let _ = display_check(&mut e.regs, "at Windows Mobile's clock");
            clock_readback(&mut e.regs);
        }
        let clk0 = e.regs.read32(MBXCLK);
        if e.regs.read32(PLLSTAT) & 4 != 0 {
            rp_drv_mbx_display::clock::restore(&mut e.regs, with_3d(clk0));
            rt::delay_us(1_000);
            e.reset(rt::delay_us);
            ready = pm_init(&mut e, vram);
        }
        log!(
            "MBXCLK 0x{clk0:x} -> 0x{:x}, parameter memory ready: {ready}",
            e.regs.read32(MBXCLK)
        );
        if ready && TEST_G14 {
            test_g14(&mut e, vram);
        } else if ready && TEST_G13 {
            test_g13(&mut e, vram);
        } else if ready && TEST_G12 {
            test_g12(&mut e, vram);
        } else if ready && TEST_G11 {
            test_g11(&mut e, vram);
        } else if ready && TEST_G10 {
            test_g10(&mut e, vram);
        } else if ready && TEST_G9 {
            test_g9(&mut e, vram);
        } else if ready && TEST_G8 {
            test_g8(&mut e, vram);
        } else if ready && TEST_G7 {
            test_g7(&mut e, vram);
        } else if ready && TEST_G6 {
            test_g6(&mut e, vram);
        } else if ready && TEST_G5 {
            test_g5(&mut e, vram, inherited);
        } else if ready && TEST_G4 {
            test_g4(&mut e, vram);
        } else if ready && TEST_G3 {
            test_g3(&mut e, vram);
        } else if ready && TEST_G2 {
            test_g2(&mut e, vram);
        } else if ready && TEST_G {
            test_g(&mut e, vram);
        } else if ready {
            let fill = rp_image::need(
                cmd::fill(
                    Format::BPP32,
                    TARGET,
                    STRIDE,
                    (0, 0, W as u16, H as u16),
                    GREY,
                ),
                "a 2D rectangle",
            );
            let _ = e.submit(&fill).and_then(|()| e.finish(vram, 200_000));
            say(
                vram,
                &["RP-GLESFEAT A D L M", "STEP 2 OF 5", "A: ALPHA TEST"],
            );
            log!("A: alpha test, red square with vertex alpha 64 over grey");
            alpha(&mut e, vram);
            say(
                vram,
                &["RP-GLESFEAT A D L M", "STEP 3 OF 5", "D: DEPTH LESS"],
            );
            log!("D: depth, red z 0.5 then blue z 0.3 / 0.8, writes on");
            depth(&mut e, vram);
            say(
                vram,
                &[
                    "RP-GLESFEAT A D L M",
                    "STEP 4 OF 5",
                    "L: 8-BIT GREY TEXTURES",
                ],
            );
            log!("L: 8-bit grey textures (code 10)");
            luminance(&mut e, vram);
            say(
                vram,
                &["RP-GLESFEAT A D L M", "STEP 5 OF 5", "M: TWO TEXTURE UNITS"],
            );
            log!("M: the second texture unit through the TA (0x80 x 0x40)");
            two_units(&mut e, vram);
            // The last render on screen until a button.
            let _ = show(
                &mut e.regs,
                Plane {
                    format: PixelFormat::Argb8888,
                    width: W as u16,
                    height: H as u16,
                    offset: TARGET,
                    stride: STRIDE,
                },
            );
        } else {
            say(
                vram,
                &["RP-GLESFEAT", "NO PARAMETER MEMORY", "PRESS A BUTTON"],
            );
        }
        let keypad = arm_keypad();
        log!(
            "{MARKER}: {} - {} until a button",
            if ready { "DONE" } else { "no parameter memory" },
            if TEST_G {
                "the summary shown"
            } else {
                "the last render shown"
            }
        );
        // Test G's summary stays up a minute: long enough to read.
        hold_until_button(keypad, if TEST_G { 60_000_000 } else { 10_000_000 });
        for (off, v) in saved {
            e.regs.write32(off, v);
        }
        e.reset(rt::delay_us);
        rp_drv_mbx_display::clock::restore(&mut e.regs, clk0);
        rp_image::finish()
    }

    /// `plane` scanned out; the display's registers as they were.
    fn show(regs: &mut Mmio, plane: Plane) -> [(usize, u32); 5] {
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

    fn arm_keypad() -> Option<Keypad<Mmio>> {
        let kp = rt::take_keypad()?;
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        Some(keypad)
    }

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
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::alpha_word;

    #[test]
    fn the_alpha_word_matches_every_captured_one() {
        // Dell's objects ("always", reference 0) and DoomGLES's textured
        // ones ("always", reference 100), captured on the device.
        assert_eq!(alpha_word(7, 0), 0xE000_0000);
        assert_eq!(alpha_word(7, 100), 0xEC80_0000);
        // Version 23's depth "less" blue square: "alpha < 0".
        assert_eq!(alpha_word(1, 0), 1 << 29);
    }
}

/// Test G's pure parts: the TA draws, way 3's list word, the tally.
#[cfg(test)]
mod g_tests {
    use super::*;
    use rp_drv_mbx3d as m3;

    #[test]
    fn the_constants_agree_with_the_driver_and_dell() {
        // Dell's smooth TSP is Dell's flat one with Gouraud's bit, bit 8
        // clear (a colour per vertex).
        assert_eq!(TSP_SMOOTH, m3::TSP_DELL | m3::TSP_GOURAUD);
        assert_eq!(TSP_SMOOTH & 1 << 8, 0);
        // The untextured state block is the GLES clear's.
        assert_eq!(m3::ta::clear_stream(480, 640)[0], TA_STATE);
        assert_eq!(TA_STRIP, m3::ta::STRIP);
        // Our flat list word carries layout 4, the textured one 8.
        assert_eq!((m3::STRIP_ONE_TRIANGLE & LAYOUT) >> 21, 4);
        assert_eq!((m3::tex::STRIP_GAME & LAYOUT) >> 21, 8);
        assert_eq!(MASK | LAYOUT, FLAGS);
        assert_eq!(MASK & LAYOUT, 0);
    }

    #[test]
    fn a_ta_draw_is_the_state_block_the_strip_and_five_words_a_vertex() {
        let clip = m3::ta::clip(480, 640);
        let (s, n) = ta_draw(TSP_SMOOTH, m3::ISP_ALWAYS, clip, &TRIS[0]);
        assert_eq!(n, 6 + 3 * TA_VERTEX_WORDS);
        assert_eq!(
            s[..6],
            [TA_STATE, TSP_SMOOTH, m3::ISP_ALWAYS, clip, 0, TA_STRIP]
        );
        let one = 1.0f32.to_bits();
        // Top left, top right, bottom left: x, y, z, 1/w, colour.
        assert_eq!(s[6..11], [0, 0, one, one, 0xFF00_0000]);
        assert_eq!(s[11..16], [480.0f32.to_bits(), 0, one, one, 0xFFFF_FFFF]);
        assert_eq!(s[16..21], [0, 640.0f32.to_bits(), one, one, 0xFFFF_E000]);
        assert!(s[n..].iter().all(|&w| w == 0));
        // A strip of four fills the buffer; a fifth corner is never sent.
        assert_eq!(ta_draw(TSP_SMOOTH, 0, clip, &[0, 1, 3, 2]).1, TA_DRAW_MAX);
        assert_eq!(
            ta_draw(TSP_SMOOTH, 0, clip, &[0, 1, 3, 2, 0]).1,
            TA_DRAW_MAX
        );
        assert_eq!(ta_draw(TSP_SMOOTH, 0, clip, &[]).1, 6);
    }

    #[test]
    fn the_two_triangles_cover_the_quad_once() {
        // Every corner used; the shared edge is the diagonal 1-3.
        let mut used = [0; 4];
        for t in TRIS {
            for k in t {
                used[k] += 1;
            }
        }
        assert_eq!(used, [1, 2, 1, 2]);
        // The quad is the whole target.
        assert_eq!((QUAD[2].0, QUAD[2].1), (480.0, 640.0));
    }

    #[test]
    fn way_3_takes_the_tas_flags_and_keeps_our_offset() {
        let ours = m3::STRIP_ONE_TRIANGLE | 0x123;
        // The same triangle mask: every flag of the TA's.
        let (w, all) = adopt_flags(ours, 0x4120_0000 | 0x1_FFFF);
        assert!(all);
        assert_eq!(w, 0x4120_0123);
        // A different mask (two triangles): the layout alone.
        let (w, all) = adopt_flags(ours, 0x6100_0000);
        assert!(!all);
        assert_eq!(w, 0x4100_0123);
        // Layout 0 (the clear's): ours goes to 0, nothing else moves.
        let (w, _) = adopt_flags(ours, 0x8000_0000);
        assert_eq!(w, 0x4000_0123);
        // The TA's own word back: unchanged.
        assert_eq!(adopt_flags(ours, ours), (ours, true));
    }

    #[test]
    fn the_tally_counts_steps_wraps_and_unwritten_words() {
        let mut t = Tally::default();
        t.add(0, 0xFF10_2030, 0xFF10_2030);
        assert_eq!(t, Tally::default());
        // One step in one channel, either way.
        t.add(1, 0xFF10_2030, 0xFF10_2031);
        t.add(2, 0xFF10_2030, 0xFF0F_2030);
        assert_eq!((t.wrong, t.one_step, t.max_step), (2, 2, 1));
        assert_eq!(t.first, Some((1, 0xFF10_2030, 0xFF10_2031)));
        // White wrapped to black in red: 255; alpha counts too.
        t.add(3, 0xFFFF_FFFF, 0xFF00_FFFF);
        t.add(4, 0xFF00_0000, 0xFE00_0000);
        assert_eq!((t.wrong, t.one_step, t.max_step), (4, 3, 255));
        // A word the render never wrote.
        t.add(5, 0xFF00_0000, SENTINEL);
        assert_eq!((t.wrong, t.unwritten), (5, 1));
        // Merged: sums, the larger step, the first kept.
        let mut u = Tally::default();
        u.add(9, 1, 3);
        u.merge(&t);
        assert_eq!(
            (u.wrong, u.one_step, u.unwritten, u.max_step),
            (6, 3, 1, 255)
        );
        assert_eq!(u.first, Some((9, 1, 3)));
        // The larger step kept whichever side holds it.
        let mut w = t;
        let mut small = Tally::default();
        small.add(0, 1, 3);
        w.merge(&small);
        assert_eq!(w.max_step, 255);
        let mut v = Tally::default();
        v.merge(&t);
        assert_eq!(v, t);
    }

    #[test]
    fn the_sentinel_is_no_colour_of_the_quad() {
        assert!(QUAD.iter().all(|&(_, _, c)| c >> 24 == 0xFF));
        assert_ne!(SENTINEL >> 24, 0xFF);
    }
}

/// Test G2's pure parts, held to the words the TA wrote on the X51v
/// (test G, `local/rp-glesfeat-G1.txt`, 2026-10-23).
#[cfg(test)]
mod g2_tests {
    use super::*;
    use rp_drv_mbx3d as m3;

    #[test]
    fn the_quad_object_is_the_tas_strip_of_four_word_for_word() {
        // Way 4's object at 0x626280 (its first 22 words).
        let ta: [u32; QUAD_WORDS] = [
            0xe000_0000,
            0x2260_0e80,
            0,
            0,
            0x43f0_0000,
            0,
            0,
            0x4420_0000,
            0x43f0_0000,
            0x4420_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0xff00_0000,
            0xffff_ffff,
            0xffff_e000,
            0xff10_20a0,
        ];
        assert_eq!(quad_object(m3::ISP_ALWAYS, TSP_SMOOTH, 1.0), ta);
        // Its list word 0x60a098a0: two triangles, layout 5.
        assert_eq!(0x60a0_98a0 & FLAGS, QUAD_FLAGS);
        assert_eq!((QUAD_FLAGS & LAYOUT) >> 21, 5);
        // z 0: only the four z words change.
        let z0 = quad_object(m3::ISP_ALWAYS, TSP_SMOOTH, 0.0);
        for (k, (&a, &b)) in z0.iter().zip(ta.iter()).enumerate() {
            assert_eq!(a == b, !(10..14).contains(&k), "word {k}");
        }
    }

    #[test]
    fn the_clear_object_is_the_tas_word_for_word() {
        // Way 4's clear at 0x64ba48 and its list word 0x40012e92.
        let ta = [
            0x2220_6f80,
            0,
            0,
            0x4470_0000,
            0,
            0,
            0x44a0_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
        ];
        assert_eq!(clear_object(480, 640), ta);
        assert_eq!(0x4001_2e92 & FLAGS, CLEAR_FLAGS);
        assert_eq!(CLEAR_FLAGS & LAYOUT, 0);
        // The clear stream's TSP is the object's.
        assert_eq!(m3::ta::clear_stream(480, 640)[1], TSP_CLEAR);
    }

    #[test]
    fn the_tas_gouraud_triangle_is_ours_but_its_z() {
        // Way 1's second object at 0x626100, list word 0x40809840.
        let ta = [
            0xe000_0000,
            0x2260_0e80,
            0,
            0,
            0x43f0_0000,
            0,
            0,
            0x4420_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0xff00_0000,
            0xffff_ffff,
            0xffff_e000,
        ];
        let v = |k: usize, z| m3::Vertex {
            x: QUAD[k].0,
            y: QUAD[k].1,
            z,
            rhw: 1.0,
            argb: QUAD[k].2,
        };
        let ours = |z| m3::triangle(m3::ISP_ALWAYS, TSP_SMOOTH, &TRIS[0].map(|k| v(k, z)));
        assert_eq!(ours(1.0), ta);
        assert_ne!(ours(0.0), ta);
        assert_eq!(0x4080_9840 & FLAGS, m3::STRIP_ONE_TRIANGLE);
    }

    #[test]
    fn a_triangle_touches_the_tiles_it_covers_and_no_others() {
        let tile = |c: u32, r: u32| {
            let (x, y) = ((c * 8) as f32, (r * 16) as f32);
            (x, y, x + 8.0, y + 16.0)
        };
        let p = |k: usize| (QUAD[k].0, QUAD[k].1);
        let (a, b) = (TRIS[0].map(p), TRIS[1].map(p));
        // The corners: each half holds its own.
        assert!(touches(a, tile(0, 0)) && !touches(b, tile(0, 0)));
        assert!(touches(b, tile(59, 39)) && !touches(a, tile(59, 39)));
        // Both on the diagonal (480, 0)-(0, 640): the top-right and
        // bottom-left corners' tiles, and one across its middle.
        for t in [tile(59, 0), tile(0, 39), tile(30, 19)] {
            assert!(touches(a, t) && touches(b, t));
        }
        // Every tile touched by one or the other (the quad covers all).
        for r in 0..40 {
            for c in 0..60 {
                assert!(
                    touches(a, tile(c, r)) || touches(b, tile(c, r)),
                    "tile {c},{r}"
                );
            }
        }
        // Either winding; a tile well outside a small triangle.
        let small = [(10.0, 10.0), (20.0, 10.0), (10.0, 20.0)];
        let flipped = [small[0], small[2], small[1]];
        for t in [small, flipped] {
            assert!(touches(t, (8.0, 8.0, 16.0, 16.0)));
            assert!(!touches(t, (100.0, 100.0, 108.0, 116.0)));
            // Inside the box, beyond the hypotenuse.
            assert!(!touches(t, (18.0, 18.0, 19.0, 19.0)));
            // Wholly inside the triangle (no corner on any edge).
            assert!(touches(t, (11.0, 11.0, 12.0, 12.0)));
            // Thin strips past a corner: no edge separates them, only the
            // bounding box does (one across, one down).
            assert!(!touches(t, (21.0, 0.0, 22.0, 30.0)));
            assert!(!touches(t, (0.0, 21.0, 30.0, 22.0)));
        }
    }

    #[test]
    fn the_tally_places_wrong_pixels_on_the_edge_or_inside() {
        let mut t = Tally::default();
        t.add_px((0, 107), (480, 640), 0, 0x48ee_b980);
        t.add_px((479, 5), (480, 640), 0, 1);
        t.add_px((5, 639), (480, 640), 0, 1);
        t.add_px((5, 0), (480, 640), 0, 1);
        assert_eq!((t.wrong, t.edge, t.first_inner), (4, 4, None));
        t.add_px((38, 8), (480, 640), 0xff17_1714, 0xff17_1717);
        t.add_px((1, 1), (480, 640), 0, 1);
        t.add_px((2, 2), (480, 640), 5, 5);
        assert_eq!((t.wrong, t.edge), (6, 4));
        assert_eq!(
            t.first_inner,
            Some((8 * 480 + 38, 0xff17_1714, 0xff17_1717))
        );
        assert_eq!(t.first, Some((107 * 480, 0, 0x48ee_b980)));
        let mut u = Tally::default();
        u.merge(&t);
        u.merge(&t);
        assert_eq!((u.edge, u.first_inner), (8, t.first_inner));
    }
}

/// Test G3's pure parts: Dell's colour clear as read from `libGLES_CL.dll`
/// (A08) and the settings.
#[cfg(test)]
mod g3_tests {
    use super::*;
    use rp_drv_mbx_display::clock::Speed;
    use rp_drv_mbx3d as m3;

    #[test]
    fn dells_clear_is_an_opaque_flat_quad_over_the_viewport() {
        // The clear's TSP is Dell's flat one, the same as our background's;
        // its state block names TSP, ISP and the bit-6 word (no clip).
        assert_eq!(DELL_CLEAR_TSP, m3::TSP_DELL);
        assert_eq!(DELL_CLEAR_STATE & !TA_STATE, 0);
        assert_eq!(TA_STATE & !DELL_CLEAR_STATE, 1 << 4);
        let s = dell_clear_draw(m3::ISP_ALWAYS, 480, 640, 1.0, 0xFF00_0000);
        assert_eq!(s[..5], [0x1000_0043, 0x2220_0E80, 0xE000_0000, 0, TA_STRIP]);
        let one = 1.0f32.to_bits();
        let (w, h) = (480.0f32.to_bits(), 640.0f32.to_bits());
        // Top left, bottom left, top right, bottom right.
        assert_eq!(s[5..10], [0, 0, one, one, 0xFF00_0000]);
        assert_eq!(s[10..15], [0, h, one, one, 0xFF00_0000]);
        assert_eq!(s[15..20], [w, 0, one, one, 0xFF00_0000]);
        assert_eq!(s[20..25], [w, h, one, one, 0xFF00_0000]);
        // Depth and colour go to every vertex.
        let t = dell_clear_draw(m3::ISP_ALWAYS, 480, 640, 0.25, 0x1234_5678);
        for v in 0..4 {
            assert_eq!(t[5 + 5 * v + 2], 0.25f32.to_bits());
            assert_eq!(t[5 + 5 * v + 4], 0x1234_5678);
        }
    }

    #[test]
    fn a_four_corner_object_takes_its_corners_in_the_order_given() {
        let c = dell_clear_corners(480, 640, 0xFF00_0000);
        let o = four_corner_object(m3::ISP_ALWAYS, DELL_CLEAR_TSP, 1.0, c);
        let f = |v: f32| v.to_bits();
        assert_eq!(
            o[2..10],
            [0, 0, 0, f(640.0), f(480.0), 0, f(480.0), f(640.0)]
        );
        assert!(o[10..18].iter().all(|&w| w == f(1.0)));
        assert!(o[18..22].iter().all(|&w| w == 0xFF00_0000));
        // The quad's object is the same builder in the strip's order.
        assert_eq!(
            quad_object(m3::ISP_ALWAYS, TSP_SMOOTH, 0.0),
            four_corner_object(
                m3::ISP_ALWAYS,
                TSP_SMOOTH,
                0.0,
                STRIP_ORDER.map(|k| QUAD[k])
            )
        );
    }

    #[test]
    fn the_settings_are_the_clocks_named_with_dells_memory_timing() {
        let khz: [u32; 3] = G3_PLLS.map(|(p, _)| Speed::rated(p).khz());
        assert_eq!(khz, [74_750, 99_666, 133_250]);
        // 74.75 is the A75 setting test G ran; 99.67 Windows Mobile's own
        // timing (only the cores' divider differs from Windows Mobile).
        assert_eq!(Speed::rated(G3_PLLS[0].0), Speed::A75);
        assert_eq!(Speed::rated(G3_PLLS[1].0), Speed::WM);
        // CL2 at every setting, as GC2 and the soaks (Dell's rule would
        // give CL3 at 133.25: no 3D picture in run G1 at 108 MHz, none
        // rendered in G3's first run).
        assert_eq!(G_LMTYPE, Speed::WM.lmtype);
        assert_eq!(Speed::A75.lmtype, G_LMTYPE);
        // What the image runs: the clock driver's ADR-0035 ladder.
        for (pll, _) in G3_PLLS {
            assert_eq!(Speed::ladder(pll).lmtype, G_LMTYPE);
        }
        assert_eq!(
            Speed::rated(G3_PLLS[2].0).lmtype,
            0xEC8,
            "Dell's rule, which G3 overrides"
        );
        assert!(
            G3_PLLS
                .windows(2)
                .all(|p| Speed::rated(p[0].0).khz() < Speed::rated(p[1].0).khz())
        );
    }
}

/// Test G4's pure parts: Dell's render table as `rp-wmprobe` read it.
#[cfg(test)]
mod g4_tests {
    use super::*;
    use rp_drv_mbx3d as m3;

    #[test]
    fn dells_table_is_the_probes_reading_and_differs_from_ours_in_every_word() {
        // rp-wmprobe run 2 (local/rp-wmprobe-5.txt), Dell's driver at
        // 74.75 MHz / 1: "run 2 after: MBX 0x610 0x00030001" and so on.
        let probe: [(usize, u32); 9] = [
            (0x610, 0x0003_0001),
            (0x614, 0x1e3c_e500),
            (0x618, 0x1e3c_e500),
            (0x61c, 0x0002_0007),
            (0x648, 0x0000_0001),
            (0x650, 0x0000_0001),
            (0x660, 0x0000_00f0),
            (0x6d8, 0x0900_0000),
            (0x6dc, 0x3f80_0001),
        ];
        for &(off, v) in DELL_TABLE.iter().chain(DELL_TARGET16.iter()) {
            assert!(probe.contains(&(off, v)), "0x{off:x} 0x{v:x}");
        }
        assert_eq!(DELL_TABLE.len() + DELL_TARGET16.len(), probe.len());
        // Every word is one our start table writes (the image replaces it,
        // never adds it), and differs from ours (else it isn't a test).
        let ours = m3::start_table(0, 0, 480, 640, 480 * 4);
        for &(off, v) in DELL_TABLE.iter().chain(DELL_TARGET16.iter()) {
            let mine = ours.iter().find(|p| p.0 == off);
            assert!(mine.is_some(), "0x{off:x} not in our table");
            assert_ne!(mine.map(|p| p.1), Some(v), "0x{off:x} the same as ours");
        }
        // Never the start register.
        assert!(!DELL_TABLE.iter().any(|p| p.0 == m3::START));
    }
}

/// Test G5's pure parts: Dell's state as `rp-wmprobe` v3 read it, his
/// `NINTPW` rule, and the halving.
#[cfg(test)]
mod g5_tests {
    use super::*;
    use rp_drv_mbx_display::clock::Speed;
    use rp_drv_mbx3d as m3;

    /// Run 2's state in the probe's log, if this checkout has it (traces
    /// stay local: docs/CLEANROOM.md): `(block, offset) -> value`.
    fn probe_state() -> Option<Vec<(String, usize, u32)>> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../local/rp-wmprobe-6.txt");
        let log = std::fs::read_to_string(path).ok()?;
        // Present but unreadable is a failure, never a skip.
        let from = log
            .rfind("run 2 after: state follows")
            .expect("no run 2 state in the log");
        let mut out = Vec::new();
        for l in log[from..].lines().skip(1) {
            if l.ends_with("state end") {
                break;
            }
            let rest = l.strip_prefix("run 2 after: ").expect(l);
            let block = rest[..3].to_string();
            let words: Vec<&str> = rest.split_whitespace().collect();
            let hex = |s: &str| {
                u32::from_str_radix(
                    s.trim_matches(|c| c == '(' || c == ')')
                        .trim_start_matches("0x"),
                    16,
                )
                .ok()
            };
            let v = hex(words[words.len() - 1]).expect(l);
            let off = hex(words[words.len() - 2]).expect(l) as usize;
            out.push((block, off, v));
        }
        assert_eq!(out.len(), 153, "run 2's state: 153 words");
        Some(out)
    }

    #[test]
    fn dells_state_is_the_probes_reading() {
        let Some(state) = probe_state() else {
            eprintln!("local/rp-wmprobe-6.txt not here: the reading not re-checked");
            return;
        };
        let get = |b: &str, off: usize| {
            state
                .iter()
                .find(|(bb, o, _)| bb == b && *o == off)
                .map(|e| e.2)
        };
        // Every MBX word: Dell's listed value, or 0, or an address.
        let mut nonzero = 0;
        for (b, off, v) in &state {
            if b != "MBX" || DELL_ADDRESSES.contains(off) {
                continue;
            }
            let listed = DELL_MBX.iter().find(|p| p.0 == *off).map_or(0, |p| p.1);
            assert_eq!(*v, listed, "MBX 0x{off:x}");
            nonzero += usize::from(*v != 0);
        }
        assert_eq!(nonzero, DELL_MBX.len(), "every listed word read non-zero");
        for &(off, v) in &DELL_SYS {
            assert_eq!(get("SYS", off), Some(v), "SYS 0x{off:x}");
        }
        assert_eq!(get("SYS", NINTPW), Some(nintpw(74_750)));
    }

    #[test]
    fn the_lists_are_sorted_and_apart() {
        assert!(DELL_MBX.windows(2).all(|p| p[0].0 < p[1].0));
        assert!(DELL_SYS.windows(2).all(|p| p[0].0 < p[1].0));
        for a in DELL_ADDRESSES {
            assert!(!DELL_MBX.iter().any(|p| p.0 == a), "0x{a:x}");
        }
        // Never the starts: the render's (0x680) and the TA's (0x800).
        assert!(!DELL_MBX.iter().any(|p| p.0 == m3::START || p.0 == 0x800));
    }

    #[test]
    fn the_items_are_dells_words_but_the_targets() {
        for (name, it) in DELL_ITEMS {
            if let DellItem::Mbx(off, v) = it {
                assert!(DELL_MBX.contains(&(off, v)), "{name}");
                assert!(!(0x650..=0x660).contains(&off), "{name}: the target's");
            }
        }
        let mbx = DELL_ITEMS
            .iter()
            .filter(|i| matches!(i.1, DellItem::Mbx(..)))
            .count();
        assert_eq!(mbx, DELL_MBX.len() - 4, "all but 0x650-0x660");
        assert_eq!(DELL_ALL.count_ones() as usize, DELL_ITEMS.len());
        // The words G4 couldn't send: not in our start table.
        let ours = m3::start_table(0, 0, 480, 640, 480 * 4);
        for off in [0x130, 0x6D0, 0x848, 0x85C, 0x860] {
            assert!(!ours.iter().any(|p| p.0 == off), "0x{off:x}");
        }
    }

    #[test]
    fn nintpw_is_dells_rule_at_both_clocks_read() {
        // Read under Dell: 0x1C at Windows Mobile's 99.67 MHz (run 1),
        // 0x15 at 74.75 (run 2).
        assert_eq!(nintpw(Speed::WM.khz()), 0x1C);
        assert_eq!(nintpw(74_750), 0x15);
        assert_eq!(nintpw(133_250), 38);
        // Both directions: a faster clock never fewer cycles.
        for k in (13_000..150_000).step_by(997) {
            assert!(nintpw(k) <= nintpw(k + 997));
        }
    }

    /// A check that's right when `need` is all in the mask.
    fn needs(need: u32) -> impl FnMut(u32) -> bool {
        move |m| m & need == need
    }

    #[test]
    fn narrow_finds_one_item_anywhere() {
        for bit in 0..DELL_ITEMS.len() {
            let (m, checks) = narrow(DELL_ALL, NARROW_CHECKS, needs(1 << bit));
            assert_eq!(m, 1 << bit, "item {bit}");
            assert!(checks <= 12, "item {bit}: {checks} checks");
        }
    }

    #[test]
    fn narrow_finds_a_pair_apart() {
        for (a, b) in [(0, 17), (6, 16), (3, 4), (9, 14)] {
            let need = 1 << a | 1 << b;
            let (m, checks) = narrow(DELL_ALL, NARROW_CHECKS, needs(need));
            assert_eq!(m, need, "items {a}, {b}");
            assert!(checks <= NARROW_CHECKS);
        }
    }

    #[test]
    fn narrow_never_checks_the_empty_mask_and_keeps_the_limit() {
        let mut seen = Vec::new();
        let (m, checks) = narrow(DELL_ALL, NARROW_CHECKS, |t| {
            seen.push(t);
            false
        });
        // Nothing can go: all of it is needed.
        assert_eq!(m, DELL_ALL);
        assert!(!seen.contains(&0));
        assert!(seen.iter().all(|&t| t & !DELL_ALL == 0));
        assert_eq!(checks as usize, seen.len());
        assert!(checks <= NARROW_CHECKS);
        let (_, few) = narrow(DELL_ALL, 3, |_| false);
        assert_eq!(few, 3);
        // A single item is its own answer, with no check.
        assert_eq!(narrow(1 << 5, NARROW_CHECKS, |_| true), (1 << 5, 0));
    }
}

/// Test G6's pure parts: the layouts.
#[cfg(test)]
mod g6_tests {
    use super::*;
    use rp_drv_mbx3d as m3;

    const TILES: u32 = (480 / m3::TILE_W) * (640 / m3::TILE_H);

    fn overlap(a: (usize, usize), b: (usize, usize)) -> bool {
        a.0 < b.0 + b.1 && b.0 < a.0 + a.1
    }

    #[test]
    fn no_buffer_overlaps_another_or_the_images_own() {
        // The image's own areas in G6: the label, the reference (ARGB8888
        // size, the larger), the fence.
        let fixed = [
            (G6_LABEL, 480 * 640 * 2, "label"),
            (G6_REF, 480 * 640 * 4, "reference"),
            (0x00FF_F000, 0x1000, "fence"),
        ];
        for (k, l) in G6_LAYOUTS.iter().enumerate() {
            let spans = l.spans(480, 640, TILES);
            for (i, a) in spans.iter().enumerate() {
                assert!(a.0 + a.1 <= 16 << 20, "layout {k}: {} past 16 MiB", a.2);
                for b in &spans[i + 1..] {
                    // Dell's TA writes the region array the render reads:
                    // the same place by design, never another buffer's.
                    let same = a.2.contains("region") && b.2.contains("region") && a.0 == b.0;
                    assert!(
                        same || !overlap((a.0, a.1), (b.0, b.1)),
                        "layout {k}: {} over {}",
                        a.2,
                        b.2
                    );
                }
                for f in &fixed {
                    assert!(
                        !overlap((a.0, a.1), (f.0, f.1)),
                        "layout {k}: {} over the {}",
                        a.2,
                        f.2
                    );
                }
            }
        }
        assert!(!overlap((G6_LABEL, 480 * 640 * 2), (G6_REF, 480 * 640 * 4)));
    }

    #[test]
    fn ours_is_every_earlier_tests_layout() {
        assert_eq!(OURS_LAYOUT.target, 0x0020_0000);
        assert_eq!(
            (OURS_LAYOUT.pm, OURS_LAYOUT.pm_size),
            (0x0060_0000, 0x0010_0000)
        );
        assert_eq!(OURS_LAYOUT.pm_table, 0x0070_0000);
        assert_eq!(OURS_LAYOUT.regions, 0x00D1_0000);
        assert!(!OURS_LAYOUT.rgb565);
        assert_eq!(G6_LAYOUTS[0], OURS_LAYOUT);
        // Ways 3 and 4 differ only in the flow.
        assert_eq!(G6_LAYOUTS[2], G6_LAYOUTS[3]);
    }

    #[test]
    fn dells_places_are_the_probes_reading() {
        // rp-wmprobe v3 run 2 (local/rp-wmprobe-6.txt), Dell's driver at
        // 74.75 / 1: "run 2 after: MBX 0x824 0x001f8000" and so on.
        let probe: [(usize, u32); 7] = [
            (0x824, 0x001f_8000),
            (0x828, 0x0000_022c),
            (0x82c, 0x0000_0a2a),
            (0x83c, 0x0022_b000),
            (0x840, 0x00f4_0000),
            (0x844, 0x00f3_5000),
            (0x65c, 0x00e9_d000),
        ];
        let l = DELL_LAYOUT;
        // His parameter memory as our set-up writes it: the same four words.
        let ours = m3::pm_init_table(l.pm as u32, l.pm_size, l.pm_table as u32);
        for &(off, v) in &probe[..4] {
            assert_eq!(
                ours.iter().find(|p| p.0 == off).map(|p| p.1),
                Some(v),
                "0x{off:x}"
            );
        }
        assert_eq!(l.tail as u32, probe[4].1);
        assert_eq!(l.ta_regions as u32, probe[5].1);
        assert_eq!(
            l.regions as u32, probe[5].1,
            "0x608 holds the same (run 2: 0x00f35000)"
        );
        assert_eq!(l.target as u32, probe[6].1);
        assert!(l.rgb565, "0x650 1");
        // Way 2 takes only the parameter memory.
        assert_eq!(
            (
                DELL_PM_LAYOUT.pm,
                DELL_PM_LAYOUT.pm_size,
                DELL_PM_LAYOUT.pm_table
            ),
            (l.pm, l.pm_size, l.pm_table)
        );
        assert_eq!(DELL_PM_LAYOUT.regions, OURS_LAYOUT.regions);
        assert!(!DELL_PM_LAYOUT.rgb565);
    }
}

/// Test G7's pure parts: the layout a frame's registers give, and where
/// the image's own buffers sit against Dell's captured frame.
#[cfg(test)]
mod g7_tests {
    use super::*;

    /// Dell's registers after his divide-1 draws (`rp-wmprobe` v4 run 2,
    /// `local/rp-wmprobe-7.txt`): the ones a layout reads.
    const DELL: [(usize, u32); 8] = [
        (0x608, 0x001D_6000),
        (0x650, 1),
        (0x65C, 0x00B2_7000),
        (0x824, 0x001C_0000),
        (0x828, 0x292),
        (0x82C, 0xA90),
        (0x83C, 0x0029_1000),
        (0x840, 0x001E_0000),
    ];

    #[test]
    fn dells_layout_is_read_from_his_registers() {
        let b = frame::write(&DELL, &[], (0, 0));
        let f = frame::parse(&b).expect("parses");
        let l = frame_layout(&f).expect("a layout");
        assert_eq!(l.target, 0x00B2_7000);
        assert!(l.rgb565);
        assert_eq!(
            (l.pm, l.pm_size, l.pm_table),
            (0x0029_1000, 8 << 20, 0x001C_0000)
        );
        assert_eq!(l.regions, 0x001D_6000);
        assert_eq!(l.tail, 0x001E_0000);
        assert_eq!(l.ta_regions, OURS_LAYOUT.ta_regions, "0x844 absent: ours");
        // Our set-up of his parameter memory writes his words exactly.
        let t = rp_drv_mbx3d::pm_init_table(l.pm as u32, l.pm_size, l.pm_table as u32);
        for &(off, v) in &DELL[3..7] {
            assert_eq!(
                t.iter().find(|p| p.0 == off).map(|p| p.1),
                Some(v),
                "0x{off:x}"
            );
        }
        // A register missing, or the pages reversed: no layout.
        let short = frame::write(&DELL[1..], &[], (0, 0));
        assert!(frame_layout(&frame::parse(&short).unwrap()).is_none());
        let mut rev = DELL;
        rev[4].1 = 0xA91;
        let b = frame::write(&rev, &[], (0, 0));
        assert!(frame_layout(&frame::parse(&b).unwrap()).is_none());
    }

    #[test]
    fn the_images_own_buffers_are_clear_of_dells_frame_and_ours() {
        let dell = frame_layout(&frame::parse(&frame::write(&DELL, &[], (0, 0))).unwrap()).unwrap();
        let own = [
            (G7_REF, 480 * 640 * 4),
            (G6_LABEL, 480 * 640 * 2),
            (0x00FF_F000, 0x1000),
        ];
        let tiles = (480 / rp_drv_mbx3d::TILE_W) * (640 / rp_drv_mbx3d::TILE_H);
        for l in [dell, OURS_LAYOUT] {
            for s in &l.spans(480, 640, tiles)[..4] {
                for o in own {
                    assert!(
                        !(s.0 < o.0 + o.1 && o.0 < s.0 + s.1),
                        "{} at 0x{:x} over 0x{:x}",
                        s.2,
                        s.0,
                        o.0
                    );
                }
            }
        }
        assert!(G7_REF + 480 * 640 * 4 <= 16 << 20);
    }

    #[test]
    fn the_local_frame_file_if_here_parses_and_lies_clear() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../local/g7-dell-frame.bin");
        let Ok(b) = std::fs::read(path) else {
            eprintln!("local/g7-dell-frame.bin not here: not re-checked");
            return;
        };
        let f = frame::parse(&b).expect("Dell's frame parses");
        let l = frame_layout(&f).expect("and gives a layout");
        assert_eq!(
            (l.target, l.pm, l.regions),
            (0x00B2_7000, 0x0029_1000, 0x001D_6000)
        );
        for (from, bytes) in [
            (G7_REF, 480 * 640 * 4),
            (G6_LABEL, 480 * 640 * 2),
            (0x00FF_F000, 0x1000),
        ] {
            assert!(!f.touches(from, bytes), "a segment over 0x{from:x}");
        }
        assert_eq!(f.picture_words, 480 * 640 / 2);
        // Never the starts or the status.
        assert!(f.regs().all(|(o, _)| !frame::NEVER.contains(&o)));
    }
}

/// Test G8's pure parts.
#[cfg(test)]
mod g8_tests {
    use super::*;

    #[test]
    fn the_ways_cover_both_waits_for_both_scenes_and_the_control() {
        assert_eq!(
            G8_WAYS[0],
            (false, Wait::Tight),
            "the control first: ours as before"
        );
        for scene in [false, true] {
            for w in [Wait::Quiet, Wait::EveryMs] {
                assert!(G8_WAYS.contains(&(scene, w)), "{scene} {w:?}");
            }
        }
        // Dell's frame is never polled tightly here: G7 did that.
        assert!(!G8_WAYS.contains(&(true, Wait::Tight)));
    }

    #[test]
    fn the_quiet_wait_outlasts_every_render_seen() {
        // The slowest render measured at divide 1: Dell's frame, 19.94 ms
        // at 74.75 / 1 (G7); at Windows Mobile's clock 19.62 ms. Twice that
        // and still under the 200 ms every render is allowed.
        assert!(QUIET_US >= 2 * 19_940);
        assert!(QUIET_US < 200_000);
    }
}

/// Test G9's pure parts.
#[cfg(test)]
mod g9_tests {
    use super::*;

    #[test]
    fn the_system_words_are_known_registers_never_a_hole_that_froze() {
        assert!(
            SYS_WORDS.windows(2).all(|w| w[0] < w[1]),
            "sorted, each once"
        );
        assert!(
            SYS_WORDS.iter().all(|o| o % 4 == 0 && *o < 0x2000),
            "below the display"
        );
        for bad in SYS_FREEZE.iter().chain(&[0x08C, 0x9C0]) {
            assert!(
                !SYS_WORDS.contains(bad),
                "0x{bad:x} froze or stalled a reader"
            );
        }
        // Every register Dell's driver writes on the 3D path is in it
        // (docs/hw/2700g.md "Dell's register writes").
        for w in [
            0x000, 0x014, 0x018, 0x02C, 0x038, 0x040, 0x04C, 0x050, 0x054, 0x058, 0x064, 0x0C0,
            0x0C4, 0x0C8, 0x304, 0x1004, 0x1008, 0x1014, 0x1018, 0x101C,
        ] {
            assert!(SYS_WORDS.contains(&w), "0x{w:x}");
        }
    }
}

/// Test G12's pure parts.
#[cfg(test)]
mod g12_tests {
    use super::*;

    #[test]
    fn the_loader_only_clocks_are_g9s_difference_exactly() {
        // CKEN under the loader and under Windows Mobile (G9: the loader's
        // dump and rp-wmprobe v5's, 2026-10-23).
        let (loader, wm) = (0x01D9_1EE3u32, 0x00D9_1AA3u32);
        assert_eq!(loader ^ wm, LOADER_ONLY_CLOCKS);
        assert_eq!(
            loader & !LOADER_ONLY_CLOCKS,
            wm,
            "off, the loader's is Windows Mobile's"
        );
        // Never the USB client's (bit 11: the loader's link) or the
        // internal memory's (bit 20).
        assert_eq!(LOADER_ONLY_CLOCKS & (1 << 11 | 1 << 20), 0);
    }

    #[test]
    fn the_silent_wait_outlasts_a_render_at_any_cpu_speed_and_ends_in_time() {
        // At least one cycle a turn at 624 MHz, at most four at 208 MHz:
        // past the slowest render (19.94 ms), inside the 200 ms allowed
        // after the check, nearly.
        let fastest_ms = SILENT_SPINS as u64 / 624_000;
        let slowest_ms = 4 * SILENT_SPINS as u64 / 208_000;
        assert!(fastest_ms >= 19, "{fastest_ms} ms");
        assert!(slowest_ms <= 250, "{slowest_ms} ms");
        assert!(G12_WAYS.iter().filter(|w| w.0).count() == 4);
        assert_eq!(G12_WAYS[0], (false, false, false), "ours as before first");
    }
}

/// Tests G5's and G14's start: Dell's divide 1 told from every other.
#[cfg(test)]
mod g14_tests {
    use super::*;

    #[test]
    fn dells_divide_1_is_his_boot_clock_only() {
        // As the image found the chip under Dell's CoreClock (G5,
        // 2026-10-23: MBXCLK 0x1, the 2D clock on, the 3D off) and as his
        // driver leaves it after drawing (rp-wmprobe: 0x2), the PLL locked.
        assert!(dells_divide_1(0x1, 0x4));
        assert!(dells_divide_1(0x2, 0x4));
        assert!(dells_divide_1(0x3, 0x5));
        // Windows Mobile's divide 2 (0x5, 0x6), the emulator's start (every
        // clock off), and a PLL not locked: not his.
        for (clk, pll) in [(0x5, 0x4), (0x6, 0x4), (0x0, 0x4), (0x2, 0x0), (0x1, 0x3)] {
            assert!(!dells_divide_1(clk, pll), "MBXCLK 0x{clk:x} PLLSTAT 0x{pll:x}");
        }
        // Divide 4 or more (bits 3-2 not zero), a clock running: not divide 1.
        assert!(!dells_divide_1(0x9, 0x4));
        assert!(!dells_divide_1(0xE, 0x4));
    }
}
