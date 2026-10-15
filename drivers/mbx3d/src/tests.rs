//! The encoder against the words Dell's driver writes (docs/hw/2700g.md,
//! "The scene bytes"), so a change to it can't drift from the proven
//! format unnoticed.

use super::*;

/// Dell's background object (`0x0211a930`): the screen-covering triangle.
#[test]
fn the_background_triangle_matches_dells_words() {
    let v = [
        Vertex::flat(0.0, 0.0, 0),
        Vertex::flat(2048.0, 0.0, 0),
        Vertex::flat(0.0, 2048.0, 0),
    ];
    let one = 1.0f32.to_bits();
    assert_eq!(
        triangle(ISP_ALWAYS, TSP_DELL, &v),
        [
            0xE000_0000,
            0x2220_0E80,
            0,
            0,
            0x4500_0000,
            0,
            0,
            0x4500_0000,
            0,
            0,
            0,
            one,
            one,
            one,
            0,
            0,
            0,
        ]
    );
}

/// Dell's minimal scene (`0x021178b8`): its x/y floats and its list.
#[test]
fn the_minimal_scene_matches_dells_words() {
    let (a, b) = (4096.0f32, 4097.0f32);
    let v = [
        Vertex {
            x: a,
            y: b,
            z: 1.0,
            rhw: 1.0,
            argb: 0,
        },
        Vertex {
            x: a,
            y: a,
            z: 1.0,
            rhw: 1.0,
            argb: 0,
        },
        Vertex {
            x: b,
            y: a,
            z: 1.0,
            rhw: 1.0,
            argb: 0,
        },
    ];
    let w = triangle(ISP_ALWAYS, TSP_DELL, &v);
    assert_eq!(
        w[2..8],
        [
            0x4580_0000,
            0x4580_0800,
            0x4580_0000,
            0x4580_0000,
            0x4580_0800,
            0x4580_0000
        ]
    );
    assert!(w[8..14].iter().all(|&x| x == 0x3F80_0000));
    // Its list: parameters 64 bytes past the list at offset 0.
    assert_eq!(one_triangle_list(64), [0x4080_0010, 0xF000_0000]);
}

#[test]
fn region_array_is_rows_outer_with_the_last_marked() {
    let mut out = [0u32; 12];
    assert_eq!(region_array(&mut out, 3, 2, 0x0080_0103), Some(12));
    let coords: [u32; 6] = core::array::from_fn(|k| out[2 * k] & !LAST_REGION);
    assert_eq!(coords, [0x000, 0x001, 0x002, 0x100, 0x101, 0x102]);
    assert!(
        out.chunks(2).all(|p| p[1] == 0x0080_0100),
        "masked to words"
    );
    assert_eq!(out[10] & LAST_REGION, LAST_REGION);
    assert!(out[..10].iter().step_by(2).all(|w| w & LAST_REGION == 0));
}

#[test]
fn region_array_refuses_what_wont_fit() {
    let mut out = [0u32; 8];
    assert_eq!(region_array(&mut out, 3, 2, 0), None, "too short");
    assert_eq!(region_array(&mut out, 0, 2, 0), None);
    assert_eq!(region_array(&mut [0u32; 600], 129, 1, 0), None, "7-bit x");
}

/// Dell's table, in Dell's order, for a 480 x 640 32 bpp target.
#[test]
fn start_table_is_dells() {
    let t = start_table(0x00C0_0003, 0x00C1_2345, 480, 640, 1920);
    assert_eq!(t[0], (0x608, 0x00C0_0000));
    assert_eq!(t[1], (0x60C, 0x00C1_2000), "4 KiB aligned");
    assert_eq!(t[4], (0x650, 6));
    assert_eq!(t[5], (0x654, 479 << 16));
    assert_eq!(t[6], (0x658, 639 << 16));
    assert_eq!(t[7], (0x660, 480));
    assert_eq!(t[8], (0x6D8, 0x0100_0000));
    assert_eq!(t[9], (0x61C, 0x0002_0004));
    assert_eq!(t[14], (START, 1), "start last");
    let regs: [usize; 15] = t.map(|(r, _)| r);
    assert!(regs[..14].iter().all(|&r| r != START));
}

/// Dell's order and arithmetic (`0x021179a8`): the table address, the
/// first page after the 4 KiB header, the last page, the base, then start.
#[test]
fn pm_init_is_dells() {
    // 1 MiB at 12 MiB, the table at 13 MiB.
    let t = pm_init_table(0x00C0_0000, 0x0010_0000, 0x00D0_0000);
    assert_eq!(t[0], (0x824, 0x00D0_0000));
    assert_eq!(t[1], (0x828, 0xC01));
    assert_eq!(t[2], (0x82C, 0xC01 + 255 - 1));
    assert_eq!(t[3], (0x83C, 0x00C0_0000));
    assert_eq!(t[4], (0x6D8, 0x0900_0000));
    assert_eq!(t[5], (0x838, 1), "start last");
}

#[test]
fn region_entry_matches_region_array() {
    let mut out = [0u32; 12];
    region_array(&mut out, 3, 2, 0x0080_0100).unwrap();
    for k in 0..6u32 {
        let e = region_entry(k % 3, k / 3, 0x0080_0100, k == 5);
        assert_eq!(e, [out[2 * k as usize], out[2 * k as usize + 1]]);
    }
}

/// DoomGLES's first textured object on the device (capture version 6, at
/// 0x2B5000), rebuilt from its numbers.
#[test]
fn a_textured_object_matches_doomgles_words() {
    let f = f32::from_bits;
    let v = [
        tex::Vertex {
            x: f(0x429E_8000),
            y: f(0x4392_5800),
            z: f(0x3F7F_9100),
            rhw: f(0x3B03_0000),
            argb: 0xFF57_5757,
            u: f(0x4202_0000),
            v: 0.0,
        },
        tex::Vertex {
            x: f(0x4207_4000),
            y: 0.0,
            z: f(0x3F7F_4100),
            rhw: f(0x3B54_0000),
            argb: 0xFF57_5757,
            u: f(0x41DF_6270),
            v: f(0x3F5F_2E00),
        },
        tex::Vertex {
            x: f(0x424C_4000),
            y: 0.0,
            z: f(0x3F7F_6000),
            rhw: f(0x3B35_0000),
            argb: 0xFF57_5757,
            u: f(0x41E1_3098),
            v: f(0x3FF7_8A00),
        },
    ];
    let texw = [tex::FORMAT_GAME, 0xCE00_3060, tex::COMBINE_GAME];
    assert_eq!(
        tex::triangle(tex::ISP_GAME, texw, tex::TSP_GAME, &v),
        [
            0xEC80_0000,
            0x0333_9600,
            0xCE00_3060,
            0x8608_6610,
            0x2264_0200,
            0x429E_8000,
            0x4392_5800,
            0x4207_4000,
            0,
            0x424C_4000,
            0,
            0x3F7F_9100,
            0x3F7F_4100,
            0x3F7F_6000,
            0x3B03_0000,
            0x3B54_0000,
            0x3B35_0000,
            0xFF57_5757,
            0x4202_0000,
            0,
            0xFF57_5757,
            0x41DF_6270,
            0x3F5F_2E00,
            0xFF57_5757,
            0x41E1_3098,
            0x3FF7_8A00,
        ]
    );
    // Its TEX1 is the address of a texture in the same capture.
    assert_eq!(tex::address(0x0018_3000), 0xCE00_3060);
}

#[test]
fn a_textured_list_points_like_doomgles() {
    // 0x41000434 pointed at the object 0x10D0 bytes into its buffer.
    let l = tex::list([0x10D0, 0x1138]);
    assert_eq!(l[..3], [0x4100_0434, 0x4100_044E, END]);
}

#[test]
fn the_frame_state_matches_the_gles_drivers_values() {
    // DoomGLES's frame: 60 x 40 tiles, depth cleared to 1.0.
    assert_eq!(frame::clear_depth(1.0), 0x3F80_0001);
    assert_eq!(frame::depth_setup(60 * 40, false), 2399 << 7 | 2);
    assert_eq!(frame::depth_setup(60 * 40, true), 2399 << 7 | 3);
    assert_eq!(frame::tile_grid(60, 40), 0x0027_003B);
    // The game's last region header: tile (59, 39), bit 29, bit 31.
    let last = region_entry(59, 39, 0, true)[0] | frame::REGION_GAME;
    assert_eq!(last, 0xA000_273B);
}

#[test]
fn the_ta_table_matches_the_gles_drivers_words() {
    let t = ta::start_table(0x00A0_0000, 0x00A2_0000, 480, 640, 60, 40);
    assert_eq!(t[0], (0x840, 0x00A0_0000));
    assert_eq!(t[1], (0x844, 0x00A2_0000));
    assert_eq!(t[3], (0x84C, 480 << 16));
    assert_eq!(t[4], (0x850, 640 << 16));
    // 1.0e-5.
    assert!((f32::from_bits(t[5].1) - 1.0e-5).abs() < 1.0e-9);
    // rows - 1 = 39 in bits 10 up, (cols - 1) & 0xFE = 58 above it.
    assert_eq!(t[7], (0x85C, 0x0004_00E1 | (39 | 58 << 9) << 10));
    assert_eq!(t[8], (ta::START, 1));
}

#[test]
fn the_clear_stream_is_the_gles_drivers() {
    let s = ta::clear_stream(480, 640);
    assert_eq!(
        s[..6],
        [0x1000_0053, 0x2220_6F80, ISP_ALWAYS, 0, 0, ta::STRIP]
    );
    // (0, 0, 1), (960, 0, 1), (0, 1280, 1).
    assert_eq!(s[6..9], [0, 0, 0x3F80_0000]);
    assert_eq!(s[9..12], [0x4470_0000, 0, 0x3F80_0000]);
    assert_eq!(s[12..15], [0, 0x44A0_0000, 0x3F80_0000]);
}

#[test]
fn a_textured_stream_has_the_state_block_then_seven_word_vertices() {
    let p = |x: f32, u: f32| tex::Vertex {
        x,
        y: 2.0,
        z: 1.0,
        rhw: 1.0,
        argb: 0xFF11_2233,
        u,
        v: 0.5,
    };
    let s = ta::textured_stream(
        tex::TSP_GAME,
        ISP_ALWAYS,
        [tex::FORMAT_GAME, 0xCE00_3060, tex::COMBINE_GAME],
        ta::clip(480, 640),
        &[p(0.0, 0.0), p(8.0, 1.0), p(4.0, 0.5)],
    );
    assert_eq!(
        s[..9],
        [
            0x1000_0057,
            tex::TSP_GAME,
            ISP_ALWAYS,
            tex::FORMAT_GAME,
            0xCE00_3060,
            tex::COMBINE_GAME,
            0x2007_4027,
            0,
            ta::STRIP
        ]
    );
    // The second vertex: x 8, y 2, z 1, 1/w 1, colour, u 1, v 0.5.
    assert_eq!(
        s[16..23],
        [
            0x4100_0000,
            0x4000_0000,
            0x3F80_0000,
            0x3F80_0000,
            0xFF11_2233,
            0x3F80_0000,
            0x3F00_0000
        ]
    );
}

#[test]
fn the_clip_word_covers_the_screen_in_16_pixel_units() {
    // x 0-29, y 0-39 (480 / 16 - 1, 640 / 16 - 1), bit 29 on.
    assert_eq!(ta::clip(480, 640), 0x2007_4027);
    assert_eq!(
        ta::dell_texture(0x00E0_0000),
        [0xA880_0000, 0x0E01_C000, 0xD688_6610]
    );
}

#[test]
fn texture_words_and_twiddling_match_version_18() {
    // Dell's TEX0 is size code 8, format 0.
    assert_eq!(tex::tex0(8, tex::ARGB1555), 0xA880_0000);
    assert_eq!(tex::tex0(8, tex::ARGB4444), 0xA880_8000);
    // The game's format field: ARGB4444.
    assert_eq!(tex::FORMAT_GAME >> tex::FORMAT_SHIFT & 7, tex::ARGB4444);
    // Pixel (41,41) of the 2048-texel ramp sampled texel (5, 3): 39 (0x27).
    assert_eq!(tex::twiddle(5, 3), 39);
    // (43,41): texel (15, 3): 175 (0xAF).
    assert_eq!(tex::twiddle(15, 3), 175);
}

#[test]
fn address_modes_match_version_22() {
    let t = tex::tex0(0, tex::RGB565);
    // Dell's high nibble is clamp/clamp.
    assert_eq!(tex::with_address(t, tex::CLAMP, tex::CLAMP), t);
    // The values version 22 tried for u: 0x2 wrap, 0x6 mirror (v clamp).
    assert_eq!(tex::with_address(t, tex::WRAP, tex::CLAMP) >> 28, 0x2);
    assert_eq!(tex::with_address(t, tex::MIRROR, tex::CLAMP) >> 28, 0x6);
}

#[test]
fn blend_and_depth_words_match_version_23() {
    // The flat square's word that blended half and half.
    assert_eq!(
        blend(TSP_DELL, factor::SRC_ALPHA, factor::INV_SRC_ALPHA),
        0x9620_2E80
    );
    // Dell's low bits are always, no write; version 23's writing modes.
    assert_eq!(depth_test(TSP_DELL, depth::ALWAYS, false), TSP_DELL);
    assert_eq!(depth_test(TSP_DELL, depth::GREATER, true), 0x2220_0800);
    assert_eq!(depth_test(TSP_DELL, depth::LESS, true), 0x2220_0200);
}
