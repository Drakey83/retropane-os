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

#[test]
fn tex1_fields_as_the_gles_driver_writes_them() {
    // Dell's point-sampled TEX1 and DoomGLES's linear-mipmap-linear one.
    assert_eq!(
        tex::tex1(0x0012_3400, tex::TEX1_LOD, 0),
        super::ta::dell_texture(0x0012_3400)[1]
    );
    assert_eq!(
        tex::tex1(0, tex::TEX1_LOD, tex::TEX1_LINEAR | tex::TEX1_MIP_LINEAR),
        tex::ADDRESS_GAME
    );
    // The GLES driver's min filter words.
    assert_eq!(tex::TEX1_MIP_NEAREST, 0x2000_0000);
    assert_eq!(tex::TEX1_LINEAR | tex::TEX1_MIP_NEAREST, 0xA000_0000);
    assert_eq!(tex::TEX1_MIP_LINEAR, 0x4000_0000);
    // The format field reaches bit 17: the GLES driver's table words.
    assert_eq!(tex::tex0(0, tex::PVRTC4) & 0x3_E000, 0x3_8000);
    assert_eq!(tex::tex0(0, tex::PVRTC2) & 0x3_E000, 0x2_0000);
    assert_eq!(tex::tex0(0, tex::ARGB8888) & 0x3_E000, 0x1_8000);
    assert_eq!(tex::tex0(0, tex::RGB565) & 0x3_E000, 0x4000);
}

#[test]
fn rectangular_twiddle_is_square_twiddle_for_squares() {
    for (x, y) in [(0, 0), (5, 3), (15, 3), (63, 63), (511, 200)] {
        assert_eq!(tex::twiddle_rect(x, y, 9, 9), tex::twiddle(x, y));
    }
}

#[test]
fn rectangular_twiddle_appends_the_longer_sides_bits() {
    // 64 x 32: x's bit 5 above the 32 x 32 square's ten bits.
    assert_eq!(tex::twiddle_rect(32, 0, 6, 5), 1 << 10);
    assert_eq!(
        tex::twiddle_rect(37, 3, 6, 5),
        (1 << 10) | tex::twiddle(5, 3)
    );
    // 32 x 64: y's bit 5 instead.
    assert_eq!(
        tex::twiddle_rect(5, 35, 5, 6),
        (1 << 10) | tex::twiddle(5, 3)
    );
    // 512 x 1024 (a 480 x 640 layer): every texel a place of its own,
    // squares of 512 x 512 one after the other down.
    let mut seen = [0u32; 512 * 1024 / 32];
    for y in 0..1024 {
        for x in 0..512 {
            let i = tex::twiddle_rect(x, y, 9, 10) as usize;
            assert!(seen[i / 32] & 1 << (i % 32) == 0, "({x},{y}) shares {i}");
            seen[i / 32] |= 1 << (i % 32);
        }
    }
    assert_eq!(tex::twiddle_rect(0, 512, 9, 10), 512 * 512);
}

#[test]
fn rectangular_tex0_carries_both_size_codes() {
    let t = tex::tex0_rect(6, 7, tex::RGB565);
    assert_eq!(t >> tex::SIZE_SHIFTS.0 & 15, 6);
    assert_eq!(t >> tex::SIZE_SHIFTS.1 & 15, 7);
    assert_eq!(
        tex::tex0_rect(3, 3, tex::ARGB4444),
        tex::tex0(3, tex::ARGB4444)
    );
}

/// The TA feed (P4 step 1) against `rp-glesfeat`'s `through_ta`, whose
/// writes drew right on the X51v (G, G2 ways 7-8, M).
mod feed {
    extern crate std;

    use core::sync::atomic::{AtomicU32, Ordering};
    use std::vec;
    use std::vec::Vec;

    use rp_drv_blit2d::{MBX, reg};
    use rp_hal_mock::{Access, MockBus};

    use super::*;

    // Starts just short of the wrap: timeouts must survive it.
    static NOW: AtomicU32 = AtomicU32::new(u32::MAX - 5_000);
    fn tick() -> u32 {
        NOW.fetch_add(1_000, Ordering::Relaxed)
    }
    const CLOCK: ta::Clock = ta::Clock {
        now: tick,
        ticks_per_ms: 1_000,
    };

    /// `rp-glesfeat`'s own layout (`OURS_LAYOUT`) and target.
    const GLESFEAT: ta::Layout = ta::Layout {
        tail: 0x00A0_0000,
        tail_bytes: 0x1_0000,
        regions: 0x00A2_0000,
        width: 480,
        height: 640,
    };

    /// FIFO levels as the status register shows them: bits 16-23, the
    /// ninth bit in bit 30.
    const fn level(words: u32) -> u32 {
        (words & 0xFF) << 16 | (words >> 8) << 30
    }

    /// `through_ta`'s writes for `draws`, as it built them: its `run` (the
    /// done bit cleared and enabled, ENABLE read as `en`), the start table,
    /// each draw then a `1` into the end window, `0x85C` with the scene-end
    /// bit, the end word, the bit cleared and ENABLE put back, `0x85C` put
    /// back.
    fn through_ta(draws: &[&[u32]], en: u32) -> Vec<(usize, u32)> {
        let t = ta::start_table(0x00A0_0000, 0x00A2_0000, 480, 640, 60, 40);
        let mut w = vec![
            (MBX + reg::CLEAR, ta::DONE),
            (MBX + reg::ENABLE, en | ta::DONE),
        ];
        w.extend(t.iter().map(|&(o, v)| (MBX + o, v)));
        let (mut n, mut ends) = (0, 0);
        for d in draws {
            for &v in d.iter() {
                w.push((ta::PORT + 4 * n, v));
                n += 1;
            }
            w.push((ta::END_PORT + 4 * ends, 1));
            ends += 1;
        }
        w.push((MBX + t[7].0, t[7].1 | ta::SCENE_END));
        w.push((ta::PORT + 4 * n, END));
        w.push((MBX + reg::CLEAR, ta::DONE));
        w.push((MBX + reg::ENABLE, en));
        w.push((MBX + t[7].0, t[7].1));
        w
    }

    fn quad(x: f32) -> [u32; 9 + 3 * ta::TEX_VERTEX_WORDS] {
        let p = |x: f32, y: f32| tex::Vertex {
            x,
            y,
            z: 0.5,
            rhw: 1.0,
            argb: 0xFFFF_FFFF,
            u: 0.0,
            v: 0.0,
        };
        ta::textured_stream(
            tex::TSP_GAME,
            ISP_ALWAYS,
            ta::dell_texture(0x00E0_0000),
            ta::clip(480, 640),
            &[p(x, 0.0), p(x + 16.0, 0.0), p(x, 16.0)],
        )
    }

    /// A bus whose FIFO is empty and whose TA says done at once.
    fn idle(en: u32) -> MockBus {
        let mut bus = MockBus::new();
        bus.preset(MBX + reg::ENABLE, en)
            .preset(MBX + reg::STATUS, ta::DONE);
        bus
    }

    #[test]
    fn four_draws_write_through_tas_proven_words() {
        let clear = ta::clear_stream(480, 640);
        let (a, b, c) = (quad(10.0), quad(100.0), quad(200.0));
        let draws: [&[u32]; 4] = [&clear, &a, &b, &c];
        let mut bus = idle(0x0000_0044);
        let fed = ta::feed(&mut bus, &GLESFEAT, ta::draws(&draws), CLOCK, 200_000);
        assert_eq!(bus.writes(), through_ta(&draws, 0x0000_0044));
        assert!(fed.done && fed.stuck.is_none());
        // 15 + 3 x 30 words, 4 end-window ones, the end word.
        assert_eq!(fed.words, 15 + 3 * 30 + 4 + 1);
        // An empty FIFO: one status read for room, the whole stream on it.
        assert_eq!(fed.level_reads, 1);
        assert_eq!(fed.level_max, 0);
        assert!(fed.ta_us >= fed.feed_us);
    }

    #[test]
    fn a_long_stream_wraps_both_windows() {
        // 2,000 words in 100 draws: the TA window's offset passes 4 KiB
        // once; 100 end-window words don't.
        let draw = [7u32; 20];
        let draws: Vec<&[u32]> = (0..100).map(|_| &draw[..]).collect();
        let mut bus = idle(0);
        let fed = ta::feed(&mut bus, &GLESFEAT, ta::draws(&draws), CLOCK, 200_000);
        assert!(fed.done);
        assert_eq!(fed.words, 2_000 + 100 + 1);
        let port: Vec<usize> = bus
            .writes()
            .iter()
            .filter(|w| (ta::PORT..ta::PORT + ta::PORT_SIZE).contains(&w.0))
            .map(|w| w.0 - ta::PORT)
            .collect();
        assert_eq!(port.len(), 2_001);
        assert!(
            port.iter()
                .enumerate()
                .all(|(k, &o)| o == 4 * k % ta::PORT_SIZE)
        );
        let ends: Vec<usize> = bus
            .writes()
            .iter()
            .filter(|w| w.0 >= ta::END_PORT)
            .map(|w| w.0 - ta::END_PORT)
            .collect();
        assert_eq!(ends, (0..100).map(|k| 4 * k).collect::<Vec<_>>());
        // 2,101 words in pieces of the FIFO's 510: five reads, no more.
        assert_eq!(fed.level_reads, 5);
    }

    #[test]
    fn the_end_window_wraps_too() {
        // 1,100 one-word draws: 1,100 end-window words pass 4 KiB.
        let draws: Vec<&[u32]> = (0..1_100).map(|_| &[5u32][..]).collect();
        let mut bus = idle(0);
        assert!(ta::feed(&mut bus, &GLESFEAT, ta::draws(&draws), CLOCK, 200_000).done);
        let ends: Vec<usize> = bus
            .writes()
            .iter()
            .filter(|w| w.0 >= ta::END_PORT)
            .map(|w| w.0 - ta::END_PORT)
            .collect();
        assert_eq!(ends.len(), 1_100);
        assert_eq!(ends[1_024], 0);
        assert_eq!(ends[1_099], 4 * 75);
    }

    #[test]
    fn a_full_fifo_is_waited_out_and_its_room_used_whole() {
        let draw = [3u32; 8];
        let mut bus = idle(0);
        // Full (0x1FF, past the 0x1FE capacity), then 2 words free, then
        // empty.
        bus.script_reads(MBX + reg::STATUS, &[level(0x1FF), level(0x1FC), 0]);
        let fed = ta::feed(&mut bus, &GLESFEAT, ta::draws(&[&draw]), CLOCK, 200_000);
        assert!(fed.done);
        assert_eq!(fed.words, 8 + 1 + 1);
        assert_eq!(fed.level_max, 0x1FE);
        // Full, 2 words free (2 written), then room for the rest: three.
        assert_eq!(fed.level_reads, 3);
        // No write went in while the FIFO was full: the first window write
        // follows the second status read, the third read the second word.
        let log = bus.log();
        let at = |n: usize| {
            log.iter()
                .enumerate()
                .filter(|(_, a)| matches!(a, Access::Write { offset, .. } if *offset >= ta::PORT))
                .nth(n)
                .unwrap()
                .0
        };
        let status_reads = |end: usize| {
            log[..end]
                .iter()
                .filter(
                    |a| matches!(a, Access::Read { offset, .. } if *offset == MBX + reg::STATUS),
                )
                .count()
        };
        assert_eq!(status_reads(at(0)), 2);
        assert_eq!(status_reads(at(2)), 3);
    }

    #[test]
    fn a_fifo_that_stays_full_ends_the_scene_without_hanging() {
        let mut bus = idle(0x10);
        bus.preset(MBX + reg::STATUS, level(0x1FE) | ta::DONE);
        let fed = ta::feed(&mut bus, &GLESFEAT, ta::draws(&[&[1, 2, 3]]), CLOCK, 50_000);
        assert_eq!(fed.stuck, Some(level(0x1FE) | ta::DONE));
        assert!(!fed.done);
        assert_eq!(fed.words, 0);
        let w = bus.writes();
        // Nothing into the windows, no scene end; the bit and 0x85C put back.
        assert!(w.iter().all(|x| x.0 < ta::PORT));
        // 0x85C written by the start table and put back, the scene end's
        // write (the same value: the table's word has bit 7 on) left out.
        let t = GLESFEAT.start_table();
        assert_eq!(w.iter().filter(|x| x.0 == MBX + 0x85C).count(), 2);
        assert_eq!(
            w[w.len() - 3..],
            [
                (MBX + reg::CLEAR, ta::DONE),
                (MBX + reg::ENABLE, 0x10),
                (MBX + 0x85C, t[7].1)
            ]
        );
    }

    #[test]
    fn a_ta_that_never_says_done_is_reported_not_waited_on() {
        let mut bus = idle(0);
        bus.preset(MBX + reg::STATUS, 0);
        let fed = ta::feed(&mut bus, &GLESFEAT, ta::draws(&[&[1]]), CLOCK, 20_000);
        assert!(!fed.done && fed.stuck.is_none());
        assert_eq!(fed.ta_us, 0);
        assert_eq!(fed.words, 3);
    }

    #[test]
    fn a_quad_stream_goes_in_as_built() {
        let state = ta::State {
            tsp: ta::TSP_DELL_TEXTURED,
            isp: ISP_ALWAYS,
            tex: ta::dell_texture(0x00E0_0000),
            clip: ta::clip(480, 640),
        };
        let q = ta::Quad {
            x0: 0.0,
            y0: 0.0,
            x1: 16.0,
            y1: 16.0,
            z: 0.5,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            argb: 0xFFFF_FFFF,
        };
        let qs = [q; 40];
        let stream = ta::quads(
            state,
            ta::Shape {
                form: ta::Form::White,
                batching: ta::Batching::Once,
                primitive: ta::Primitive::Strips,
            },
            &qs,
        );
        let want: Vec<u32> = stream
            .clone()
            .filter_map(|i| match i {
                ta::Item::Word(w) => Some(w),
                ta::Item::End => None,
            })
            .collect();
        let mut bus = idle(0);
        let fed = ta::feed(&mut bus, &GLESFEAT, stream, CLOCK, 200_000);
        assert!(fed.done);
        assert_eq!(
            fed.words as usize,
            ta::items(
                ta::Shape {
                    form: ta::Form::White,
                    batching: ta::Batching::Once,
                    primitive: ta::Primitive::Strips
                },
                40
            ) + 1
        );
        let port: Vec<u32> = bus
            .writes()
            .iter()
            .filter(|w| (ta::PORT..ta::PORT + ta::PORT_SIZE).contains(&w.0))
            .map(|w| w.1)
            .collect();
        assert_eq!(port[..port.len() - 1], want[..]);
        assert_eq!(*port.last().unwrap(), END);
        assert_eq!(
            bus.writes().iter().filter(|w| w.0 >= ta::END_PORT).count(),
            40
        );
    }

    #[test]
    fn another_writer_takes_the_window_between_the_cpus_words() {
        // A DMA's way: room asked once, 1,000 words "moved" by it (the
        // window wrapped), then the CPU's end and the scene's end after
        // them.
        let mut bus = idle(0);
        let mut f = ta::Feed::start(&mut bus, &GLESFEAT, CLOCK, 200_000);
        assert!(f.put(ta::Item::Word(9)));
        assert_eq!(f.room(), 510);
        assert_eq!(f.window_offset(), 4);
        f.advanced(500);
        assert_eq!(f.room(), 510);
        f.advanced(500);
        assert_eq!(f.window_offset(), 4 * 1_001 % ta::PORT_SIZE);
        assert!(f.put(ta::Item::End));
        let fed = f.finish();
        assert!(fed.done);
        assert_eq!(fed.words, 1 + 1_000 + 1 + 1);
        let port: Vec<(usize, u32)> = bus
            .writes()
            .into_iter()
            .filter(|w| w.0 >= ta::PORT && w.0 < ta::PORT + ta::PORT_SIZE)
            .collect();
        // The CPU's first word, then the end word where the other writer
        // left off.
        assert_eq!(
            port,
            [(ta::PORT, 9), (ta::PORT + 4 * 1_001 % ta::PORT_SIZE, END)]
        );
        assert!(bus.writes().contains(&(ta::END_PORT, 1)));
    }

    #[test]
    fn the_scene_end_bit_can_be_off_for_the_body_and_on_for_the_end() {
        let mut bus = idle(0);
        let mut f = ta::Feed::start(&mut bus, &GLESFEAT, CLOCK, 200_000);
        f.scene_end_in_body(false);
        assert!(f.put(ta::Item::Word(1)));
        assert!(f.finish().done);
        let grid = GLESFEAT.start_table()[7].1;
        let g: Vec<u32> = bus
            .writes()
            .iter()
            .filter(|w| w.0 == MBX + 0x85C)
            .map(|w| w.1)
            .collect();
        // The table's, the body's (bit 7 off), the end's (on), put back.
        assert_eq!(g, [grid, grid & !ta::SCENE_END, grid, grid]);
    }

    #[test]
    fn the_layout_gives_glesfeats_start_table() {
        assert_eq!(GLESFEAT.grid(), (60, 40));
        assert_eq!(
            GLESFEAT.start_table(),
            ta::start_table(0x00A0_0000, 0x00A2_0000, 480, 640, 60, 40)
        );
    }

    #[test]
    fn clear_zeroes_the_tail_and_the_region_array_and_stays_in_bounds() {
        let small = ta::Layout {
            tail: 0x100,
            tail_bytes: 0x40,
            regions: 0x200,
            width: 16,
            height: 32,
        };
        let mut vram = vec![0xAAu8; 0x400];
        assert!(ta::clear(&mut vram, &small));
        assert!(vram[0x100..0x140].iter().all(|&b| b == 0));
        // 2 x 2 tiles + 1, 8 bytes each: 40 bytes.
        assert!(vram[0x200..0x228].iter().all(|&b| b == 0));
        assert_eq!(vram[0x228], 0xAA);
        assert_eq!(vram[0x140], 0xAA);
        assert!(!ta::clear(&mut vram[..0x220], &small));
    }
}

/// The quad streams (P4 step 2), word by word.
mod quads {
    extern crate std;

    use std::vec::Vec;

    use super::*;
    use ta::{Batching, Form, Item};

    const STATE: ta::State = ta::State {
        tsp: ta::TSP_DELL_TEXTURED,
        isp: ISP_ALWAYS,
        tex: [0xA880_0000, 0x0E01_C000, 0xD688_6610],
        clip: 0x2007_4027,
    };

    fn strips(form: Form, batching: Batching) -> ta::Shape {
        ta::Shape {
            form,
            batching,
            primitive: ta::Primitive::Strips,
        }
    }

    fn list(form: Form, batching: Batching) -> ta::Shape {
        ta::Shape {
            form,
            batching,
            primitive: ta::Primitive::List,
        }
    }

    fn quad(x: f32, argb: u32) -> ta::Quad {
        ta::Quad {
            x0: x,
            y0: 10.0,
            x1: x + 16.0,
            y1: 26.0,
            z: 0.5,
            u0: 0.0,
            v0: 0.25,
            u1: 1.0,
            v1: 0.75,
            argb,
        }
    }

    fn words(s: &[Item]) -> Vec<u32> {
        s.iter()
            .filter_map(|i| match *i {
                Item::Word(w) => Some(w),
                Item::End => None,
            })
            .collect()
    }

    #[test]
    fn a_coloured_quad_is_the_proven_state_block_and_a_strip_of_four() {
        let q = quad(100.0, 0xFF11_2233);
        let s: Vec<Item> =
            ta::quads(STATE, strips(Form::Coloured, Batching::PerQuad), &[q]).collect();
        assert_eq!(s.len(), 8 + 1 + 4 * 7 + 1);
        assert_eq!(*s.last().unwrap(), Item::End);
        // The state block and strip word as textured_stream's (drawn on the
        // X51v, rp-glesfeat L).
        let tri = ta::textured_stream(
            STATE.tsp,
            STATE.isp,
            STATE.tex,
            STATE.clip,
            &[tex::Vertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                rhw: 1.0,
                argb: 0,
                u: 0.0,
                v: 0.0,
            }; 3],
        );
        let w = words(&s);
        assert_eq!(w[..9], tri[..9]);
        // Corners top left, top right, bottom left, bottom right, each
        // x, y, z, 1/w, colour, u, v.
        let f = |v: f32| v.to_bits();
        let corner = |x, y, u, v| [f(x), f(y), f(0.5), f(1.0), 0xFF11_2233, f(u), f(v)];
        assert_eq!(w[9..16], corner(100.0, 10.0, 0.0, 0.25));
        assert_eq!(w[16..23], corner(116.0, 10.0, 1.0, 0.25));
        assert_eq!(w[23..30], corner(100.0, 26.0, 0.0, 0.75));
        assert_eq!(w[30..37], corner(116.0, 26.0, 1.0, 0.75));
    }

    #[test]
    fn the_white_form_leaves_out_only_the_colour_word() {
        let q = quad(0.0, 0xDEAD_BEEF);
        let c = words(
            &ta::quads(STATE, strips(Form::Coloured, Batching::Once), &[q]).collect::<Vec<_>>(),
        );
        let w =
            words(&ta::quads(STATE, strips(Form::White, Batching::Once), &[q]).collect::<Vec<_>>());
        assert_eq!(w.len(), 9 + 4 * 6);
        assert!(!w.contains(&0xDEAD_BEEF));
        assert_eq!(c[..9], w[..9]);
        for k in 0..4 {
            let (cv, wv) = (&c[9 + 7 * k..16 + 7 * k], &w[9 + 6 * k..15 + 6 * k]);
            assert_eq!(cv[..4], wv[..4]);
            assert_eq!(cv[5..], wv[4..]);
        }
    }

    #[test]
    fn batching_once_sends_one_state_block_and_a_strip_and_end_a_quad() {
        let qs = [quad(0.0, 1), quad(20.0, 2), quad(40.0, 3)];
        let once: Vec<Item> =
            ta::quads(STATE, strips(Form::Coloured, Batching::Once), &qs).collect();
        let per: Vec<Item> =
            ta::quads(STATE, strips(Form::Coloured, Batching::PerQuad), &qs).collect();
        assert_eq!(once.len(), 8 + 3 * 30);
        assert_eq!(per.len(), 3 * 38);
        let masks = |s: &[Item]| {
            s.iter()
                .filter(|&&i| i == Item::Word(ta::TEXTURED_MASK))
                .count()
        };
        assert_eq!((masks(&once), masks(&per)), (1, 3));
        let ends = |s: &[Item]| s.iter().filter(|&&i| i == Item::End).count();
        assert_eq!((ends(&once), ends(&per)), (3, 3));
        // Once: the second quad's strip word right after the first's end.
        assert_eq!(once[8 + 29], Item::End);
        assert_eq!(once[8 + 30], Item::Word(ta::STRIP));
        // Per quad: each quad's items the one-quad stream's.
        let one: Vec<Item> =
            ta::quads(STATE, strips(Form::Coloured, Batching::PerQuad), &qs[1..2]).collect();
        assert_eq!(per[38..76], one[..]);
    }

    #[test]
    fn the_counts_match_the_streams_and_the_iterators_length() {
        let qs: Vec<ta::Quad> = (0..7).map(|k| quad(k as f32, 0)).collect();
        for form in [Form::Coloured, Form::White] {
            for b in [Batching::PerQuad, Batching::Once] {
                for shape in [strips(form, b), list(form, b)] {
                    for n in [0, 1, 7] {
                        let mut it = ta::quads(STATE, shape, &qs[..n]);
                        assert_eq!(it.len(), ta::items(shape, n));
                        assert_eq!(it.clone().count(), ta::items(shape, n));
                        let _ = it.nth(4);
                        assert_eq!(it.len(), ta::items(shape, n).saturating_sub(5));
                        assert_eq!(it.count(), ta::items(shape, n).saturating_sub(5));
                    }
                }
            }
        }
        // Dell's frame of 5,000 quads: 38 words a quad (190,000 items);
        // state once with the white vertex: 26 a quad.
        assert_eq!(
            ta::items(strips(Form::Coloured, Batching::PerQuad), 5_000),
            190_000
        );
        assert_eq!(
            ta::items(strips(Form::White, Batching::Once), 5_000),
            8 + 130_000
        );
    }

    #[test]
    fn a_list_batch_is_one_primitive_of_six_corners_a_quad() {
        let qs = [quad(0.0, 1), quad(20.0, 2)];
        let s: Vec<Item> = ta::quads(STATE, list(Form::Coloured, Batching::Once), &qs).collect();
        // State, the list word, 2 x 6 corners of 7 words, one end.
        assert_eq!(s.len(), 8 + 1 + 2 * 6 * 7 + 1);
        assert_eq!(s[8], Item::Word(ta::LIST));
        assert_eq!(s.iter().filter(|&&i| i == Item::End).count(), 1);
        assert_eq!(*s.last().unwrap(), Item::End);
        // Corners: top left, top right, bottom left; top right, bottom
        // right, bottom left (x, y the first two words of each).
        let w = words(&s);
        let xy = |k: usize| (f32::from_bits(w[9 + 7 * k]), f32::from_bits(w[10 + 7 * k]));
        assert_eq!(
            (0..6).map(xy).collect::<Vec<_>>(),
            [
                (0.0, 10.0),
                (16.0, 10.0),
                (0.0, 26.0),
                (16.0, 10.0),
                (16.0, 26.0),
                (0.0, 26.0)
            ]
        );
        // The second quad follows at once: no word, no end between.
        assert_eq!(xy(6), (20.0, 10.0));
        // Per quad: each its own state, list word and end.
        let per: Vec<Item> =
            ta::quads(STATE, list(Form::Coloured, Batching::PerQuad), &qs).collect();
        assert_eq!(per.len(), 2 * (8 + 1 + 6 * 7 + 1));
        assert_eq!(per.iter().filter(|&&i| i == Item::End).count(), 2);
        assert_eq!(
            ta::items(list(Form::White, Batching::Once), 5_000),
            9 + 5_000 * 36 + 1
        );
    }
}
