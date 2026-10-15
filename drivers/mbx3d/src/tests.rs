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
