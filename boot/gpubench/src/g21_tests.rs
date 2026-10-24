//! Run G21's shared part on the host: every kind drawn through the model
//! of the 3D core the same on the screen as off it, the diff and the
//! tally at their edges, the ways as the run needs them.

extern crate std;

use rp_mbx3d_render::model::{Gpu, Mem, SimClock, memory};
use rp_mbx3d_render::{
    LocalMem as _, Picture, PixelFormat, Poll, Renderer, Target, address, format,
};
use rp_vram_map::{POOL, Pool};

use super::*;

/// Every kind rendered through the model into `at` and read back.
fn rendered(at_screen: bool) -> std::vec::Vec<std::vec::Vec<u32>> {
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let mut r = Renderer::new(&mut pool, W as u16, H as u16).expect("renderer");
    // A 128 x 128 RGB565 card (code 4), any words: the place is what's
    // tested, not the texture.
    let mut tex = Texture {
        at: 0,
        code: 4,
        format: format::RGB565 as u8,
        u_mode: address::WRAP as u8,
        v_mode: address::WRAP as u8,
        linear: false,
        levels: 1,
    };
    tex.at = pool.alloc(tex.bytes(), 128).expect("texture memory");
    for i in 0..tex.bytes() / 4 {
        mem.store((tex.at + 4 * i) as usize, &[i.wrapping_mul(0x9E37_79B9)]);
    }
    let off = pool.alloc(SCREEN_BYTES, 0x1000).expect("target memory");
    let at = if at_screen { SCREEN_AT } else { off };
    assert!(
        at_screen || off >= SAFE_END,
        "the pool lies outside the safe region"
    );
    r.set_background(Some(BACKGROUND)).expect("background");
    let mut out = std::vec::Vec::new();
    for kind in 0..KINDS.len() {
        for j in 0..PLACES.len() {
            r.set(&mut mem, 1 + j, Some(&card(kind, j, tex)))
                .expect("slot");
        }
        r.invalidate_all();
        let t = Target {
            picture: Picture::packed(at, W as u16, H as u16, PixelFormat::Rgb565),
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
        out.push(
            (0..WORDS)
                .map(|k| mem.load(at as usize + 4 * k).expect("in memory"))
                .collect(),
        );
    }
    out
}

#[test]
fn every_kind_draws_the_same_on_the_screen_as_off_it() {
    let (off, on) = (rendered(false), rendered(true));
    for kind in 0..KINDS.len() {
        let (n, first, nf) = diff(|k| off[kind][k], |k| on[kind][k]);
        assert_eq!(n, 0, "{}: {:?}", KINDS[kind], &first[..nf]);
    }
    // The sentinel the image fills targets with is no word of any kind's
    // picture: a render or copy that doesn't happen can't pass.
    for kind in 0..KINDS.len() {
        assert!(!on[kind].contains(&SENTINEL), "{}", KINDS[kind]);
    }
    // The flat cards drawn (not the background alone): card 0's middle in
    // its colour, the corner the background's, as RGB565.
    let rgb565 = |c: u32| ((c >> 8 & 0xF800) | (c >> 5 & 0x07E0) | (c >> 3 & 0x001F)) as u16;
    let px = |x: u32, y: u32| {
        let w = on[0][((y * W + x) / 2) as usize];
        (if x.is_multiple_of(2) { w } else { w >> 16 }) as u16
    };
    assert_eq!(px(120, 170), rgb565(COLOURS[0]));
    assert_eq!(px(2, 2), rgb565(BACKGROUND));
    // Every kind its own picture.
    for a in 0..KINDS.len() {
        for b in a + 1..KINDS.len() {
            assert_ne!(on[a], on[b], "{} and {}", KINDS[a], KINDS[b]);
        }
    }
}

#[test]
fn diff_counts_every_word_and_places_the_first_four() {
    let a = |k: usize| k as u32;
    assert_eq!(diff(a, a).0, 0);
    // Words 0, 1, 240 (row 1), the last, and two more: six off, the first
    // four placed at their first pixel.
    let off = [0usize, 1, 240, 300, 301, WORDS - 1];
    let b = |k: usize| {
        if off.contains(&k) {
            !(k as u32)
        } else {
            k as u32
        }
    };
    let (n, first, nf) = diff(a, b);
    assert_eq!((n, nf), (6, 4));
    assert_eq!(first[0], (0, 0, 0, !0));
    assert_eq!(first[1], (2, 0, 1, !1));
    assert_eq!(first[2], (0, 1, 240, !240));
    assert_eq!(first[3], (120, 1, 300, !300));
    // Only the last word: found, at the picture's last pair of pixels.
    let c = |k: usize| if k == WORDS - 1 { 0 } else { k as u32 };
    let (n, first, nf) = diff(a, c);
    assert_eq!((n, nf), (1, 1));
    assert_eq!(first[0], (W - 2, H - 1, (WORDS - 1) as u32, 0));
}

#[test]
fn the_tally_keeps_times_misses_and_copies_apart() {
    let mut t = Tally::default();
    let none = (0, [(0, 0, 0, 0); 4], 0);
    let some = (3, [(4, 5, 6, 7); 4], 4);
    t.render(Some(900), none);
    t.render(None, none);
    t.render(Some(700), some);
    t.render(Some(800), some);
    assert_eq!((t.renders, t.unfinished, t.off, t.words), (4, 1, 2, 6));
    assert_eq!((t.render_min, t.render_mean()), (700, 800));
    // The first words from the first renders off, never more than four.
    assert_eq!(t.nfirst, 4);
    // A first render that didn't finish leaves no zero fastest.
    let mut u = Tally::default();
    u.render(None, none);
    u.render(Some(500), none);
    assert_eq!((u.render_min, u.render_mean()), (500, 500));
    u.copy(true, 5_000, 0);
    u.copy(true, 4_000, 2);
    u.copy(false, 6_000, 0);
    assert_eq!((u.copies, u.copy_min, u.copy_mean()), (3, 4_000, 5_000));
    assert_eq!((u.copies_off, u.copy_words), (2, 2));
    assert_eq!(Tally::default().render_mean(), 0);
}

#[test]
fn the_reference_is_off_screen_and_paused_and_only_a_copies() {
    assert_eq!(WAYS[0], Way::Reference);
    assert!(!WAYS[0].on_screen() && WAYS[0].paused() && !WAYS[0].copies());
    // A: off screen, the display reading, the copy in.
    assert!(!Way::CopyIn.on_screen() && !Way::CopyIn.paused() && Way::CopyIn.copies());
    // C as the owner would run it, and its place alone.
    assert!(Way::Inside.on_screen() && !Way::Inside.paused());
    assert!(Way::InsidePaused.on_screen() && Way::InsidePaused.paused());
    assert_eq!(WAYS.iter().filter(|w| w.copies()).count(), 1);
    assert!(WAYS.iter().all(|w| !(w.on_screen() && w.copies())));
    // Each way its own name in the log (the emulator's check finds them).
    for (a, wa) in WAYS.iter().enumerate() {
        for wb in &WAYS[a + 1..] {
            assert_ne!(wa.name(), wb.name());
        }
    }
    // The safe region G19's last right end (the screen inside it: a
    // const assert in g21.rs).
    assert_eq!(SAFE_END, 0xA_E000);
}
