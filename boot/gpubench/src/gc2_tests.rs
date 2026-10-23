//! Run GC2's shared part on the host: the model table held to the model,
//! the error descriptions at their edges.

extern crate std;

use std::string::String;

use rp_mbx3d_render::model::{Gpu, Mem, SimClock, memory};
use rp_mbx3d_render::{LocalMem as _, Picture, Poll, Renderer, Target};
use rp_vram_map::{POOL, Pool};

use super::*;

/// Every kind rendered through the model of the 3D core: its band
/// hashes, as the image takes them from its references.
fn model_hashes() -> [[u32; BANDS]; 9] {
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let mut r = Renderer::new(&mut pool, W as u16, H as u16).expect("renderer");
    let mut place = |n: usize, mem: &mut Mem| {
        let t = TEXTURES[n];
        let at = pool.alloc(t.bytes(), 128).expect("texture memory");
        texture_words(n, |i, w| mem.store((at + 4 * i) as usize, &[w]));
        Texture { at, ..t }
    };
    let tex = Tex {
        t565: place(0, &mut mem),
        t4444: place(1, &mut mem),
        t8888: place(2, &mut mem),
    };
    let at = pool.alloc(W * H * 4, 128).expect("target memory");
    let mut out = [[0; BANDS]; 9];
    for (k, bands) in out.iter_mut().enumerate() {
        let (bg, slots) = scene(k, tex);
        r.set_background(Some(bg)).expect("background");
        for (s, d) in slots.iter().enumerate() {
            r.set(&mut mem, s, d.as_ref()).expect("slot");
        }
        r.invalidate_all();
        let t = Target {
            picture: Picture::packed(at, W as u16, H as u16, KINDS[k].1),
            x: 0,
            y: 0,
        };
        let due = r
            .render(&mut gpu, &mut mem, &mut clock, t)
            .expect("render")
            .expect("something to render");
        clock.set(due);
        loop {
            match r.poll(&mut gpu, &mut mem, &mut clock) {
                Poll::Done(_) => break,
                Poll::Wait(t) => clock.set(t),
                p => panic!("{p:?}"),
            }
        }
        for (b, h) in bands.iter_mut().enumerate() {
            *h = (0..band_words(k)).fold(HASH_START, |h, i| {
                let a = at + 4 * (b as u32 * band_words(k) + i);
                fold(h, mem.load(a as usize).expect("in memory"))
            });
        }
    }
    out
}

#[test]
fn the_model_table_is_what_the_model_renders() {
    let got = model_hashes();
    if got != MODEL {
        let mut s = String::from("[\n");
        for bands in got {
            s.push_str("    [");
            for (b, h) in bands.iter().enumerate() {
                let sep = if b == 0 { "" } else { ", " };
                s.push_str(&std::format!("{sep}0x{h:08x}"));
            }
            s.push_str("],\n");
        }
        s.push_str("]\n");
        panic!("boot/gpubench/src/gc2_model.in is out of date; the model gives:\n{s}");
    }
}

#[test]
fn every_kind_differs_from_every_other() {
    // A scene that drew nothing, or the same as another, would make the
    // references prove less than they say.
    for (a, x) in MODEL.iter().enumerate() {
        for (b, y) in MODEL.iter().enumerate().skip(a + 1) {
            assert_ne!(x, y, "kinds {a} and {b}");
        }
    }
}

#[test]
fn the_capped_scenes_keep_every_channel_below_its_top() {
    let tex = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
    };
    for k in [7, 8] {
        let (_, slots) = scene(k, tex);
        let Some(Draw {
            paint: Paint::Gouraud(c),
            ..
        }) = slots[0]
        else {
            panic!("kind {k}: a Gouraud quad");
        };
        for argb in c {
            let ch = [argb >> 24, argb >> 16 & 0xFF, argb >> 8 & 0xFF, argb & 0xFF];
            if k == 7 {
                // RGB565: red and blue at most 30 of 31, green 62 of 63.
                assert!(
                    ch[1] >> 3 < 31 && ch[2] >> 2 < 63 && ch[3] >> 3 < 31,
                    "{argb:#010x}"
                );
            } else {
                assert!(ch.iter().all(|&v| v < 0xFF), "{argb:#010x}");
            }
        }
    }
}

#[test]
fn a_wrong_pixel_is_described_channel_by_channel() {
    let p = PixelFormat::Rgb565;
    // The device's own (FT2 fast, read 2026-10-23).
    assert_eq!(miss(p, 0xf79e, 0xf7be), Miss::Plus1, "green +1");
    assert_eq!(miss(p, 0x7bef, 0x8410), Miss::Plus1, "+1 in each, carried");
    assert_eq!(miss(p, 0x9491, 0x9492), Miss::Plus1, "blue +1");
    assert_eq!(miss(p, 0xffff, 0x0840), Miss::Wrapped, "white plus one");
    // Both directions, and each class's edges.
    assert_eq!(miss(p, 0x8410, 0x7bef), Miss::Minus1);
    assert_eq!(miss(p, 0x0000, 0x0821), Miss::Plus1);
    assert_eq!(miss(p, 0x0821, 0x0000), Miss::Minus1);
    assert_eq!(miss(p, 0x0000, 0x0002), Miss::Other, "+2");
    assert_eq!(miss(p, 0x0821, 0x1000), Miss::Other, "one up, one down");
    // Wrapped needs the top or one below it, and a drop past half.
    assert_eq!(miss(p, 0xf000, 0x0000), Miss::Wrapped, "red 30 to 0");
    assert_eq!(miss(p, 0xe800, 0x0000), Miss::Other, "red 29 to 0");
    assert_eq!(miss(p, 0xf800, 0x8000), Miss::Other, "red 31 to 16: half");
    assert_eq!(miss(p, 0xf800, 0x7800), Miss::Wrapped, "red 31 to 15");
    let q = PixelFormat::Argb8888;
    assert_eq!(miss(q, 0xFF00_0000, 0x0000_0000), Miss::Wrapped, "alpha");
    assert_eq!(miss(q, 0x1234_5678, 0x1234_5679), Miss::Plus1);
    assert_eq!(miss(q, 0x1234_5678, 0x1233_5678), Miss::Minus1);
    assert_eq!(miss(q, 0x1234_5678, 0x1234_7778), Miss::Other);
    assert_eq!(
        miss(q, 0xFFFE_FFFF, 0xFF00_0000),
        Miss::Wrapped,
        "green 254"
    );
    assert_eq!(
        miss(PixelFormat::Rgb565Dithered, 0xf79e, 0xf7be),
        Miss::Plus1
    );
}

#[test]
fn a_tally_places_and_counts_every_wrong_pixel() {
    let p = PixelFormat::Rgb565;
    let mut t = Tally::default();
    // Word 240: pixels 480 and 481, (0, 1) and (1, 1).
    t.add(p, 240, 0xf79e_ffff, 0xf7be_0840);
    // Word 241: only its high pixel, (3, 1).
    t.add(p, 241, 0x1111_2222, 0x1112_2222);
    assert_eq!(t.by, [2, 0, 1, 0]);
    assert_eq!(t.pixels(), 3);
    assert_eq!(t.bands, 1);
    assert_eq!(t.bbox, Some((0, 1, 3, 1)));
    assert_eq!(
        t.first[..t.n_first],
        [
            (0, 1, 0xffff, 0x0840),
            (1, 1, 0xf79e, 0xf7be),
            (3, 1, 0x1111, 0x1112)
        ]
    );
    // ARGB8888: a word a pixel; the last row is in the last band.
    let mut u = Tally::default();
    u.add(PixelFormat::Argb8888, W * H - 1, 0xFF00_0000, 0xFF00_0001);
    u.add(PixelFormat::Argb8888, 5, 7, 7);
    assert_eq!(u.by, [1, 0, 0, 0]);
    assert_eq!(u.bands, 1 << (BANDS - 1));
    assert_eq!(u.bbox, Some((479, 639, 479, 639)));
    // Row 16 starts band 1.
    let mut v = Tally::default();
    v.add(PixelFormat::Argb8888, 16 * W, 1, 2);
    v.add(PixelFormat::Argb8888, 16 * W - 1, 1, 2);
    assert_eq!(v.bands, 0b11);
    t.merge(&u);
    assert_eq!(t.by, [3, 0, 1, 0]);
    assert_eq!(t.bands, 1 | 1 << (BANDS - 1));
    assert_eq!(t.bbox, Some((0, 1, 479, 639)));
    assert_eq!(t.n_first, 3, "the first three kept");
    let mut e = Tally::default();
    e.merge(&u);
    assert_eq!(e.bbox, u.bbox);
    assert_eq!(e.first[..e.n_first], [(479, 639, 0xFF00_0000, 0xFF00_0001)]);
}

#[test]
fn the_textures_are_written_whole_in_order() {
    for (n, t) in TEXTURES.iter().enumerate() {
        let mut seen = 0;
        texture_words(n, |i, _| {
            assert_eq!(i, seen);
            seen += 1;
        });
        assert_eq!(seen * 4, t.bytes(), "texture {n}");
    }
    // Texel 1 of the twiddled order is (0, 1): the high half of word 0.
    texture_words(0, |i, w| {
        if i == 0 {
            assert_eq!(
                w,
                u32::from(texel565(0, 0)) | u32::from(texel565(0, 1)) << 16
            );
        }
    });
}
