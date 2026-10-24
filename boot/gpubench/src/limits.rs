//! Run LIM (`RP_GPUBENCH_LIMITS`): the 3D core's limits for the gate's P14
//! (docs/research/2700g-full-potential.md), at whatever clock the loader
//! leaves, into a 16-bit target (ADR-0023's screen), 3 renders each:
//!
//! - **objects a frame**: K separate 64 x 64 quads (32 tiles each), flat
//!   and textured, K = 1 to 63 (the renderer's 64 slots,
//!   one the background's: our design's ceiling, not the chip's);
//! - **layers a tile**: K full-screen translucent flat layers, K = 8 to 63
//!   (G2 stopped at 8);
//! - **textures a frame**: 32 textured quads sharing one texture against
//!   32 with a texture each (the texture cache switching);
//! - **the texture cache**: one full-screen quad sampling a whole texture
//!   of side 64 to 1024 (magnified, then shrunk past 1:1), point sampling;
//!   the time a tile against the side shows where the cache stops holding
//!   the texels a tile needs.
//!
//! Not here (and why): trilinear (TEX1 bit 30 hung the core in `rp-mbx3d`
//! version 25: an owner-present run), points, lines and strips (our
//! renderer draws quads; the TA path is P4's).

use super::*;

const T16: Target = Target {
    picture: PIC16,
    x: 0,
    y: 0,
};

/// K 64 x 64 quads in a 7 x 10 grid of cells, painted by `paint(k)`.
fn grid(g: &mut Rig, r: &mut Renderer, k: usize, paint: &dyn Fn(usize) -> Paint) {
    for slot in 1..rp_mbx3d_render::MAX_SLOTS {
        let d = (slot <= k).then(|| {
            let cell = (slot - 1) as i32;
            Draw::rect((cell % 7) * 68, (cell / 7) * 64, 64, 64, paint(slot - 1))
        });
        g.set(r, slot, d);
    }
}

fn clear(g: &mut Rig, r: &mut Renderer) {
    for slot in 1..rp_mbx3d_render::MAX_SLOTS {
        g.set(r, slot, None);
    }
}

#[inline(never)]
pub(super) fn run(g: &mut Rig) {
    log!(
        "LIM: the 3D core's limits (P14), 3 renders each into a 16-bit target; clock as found: COREPLL {:08X} MBXCLK {:08X}",
        g.e.regs.read32(clock::COREPLL),
        g.e.regs.read32(MBXCLK)
    );
    wm_clock_first(g);
    clock_card(
        g,
        ["RUN LIM", "THE 3D CORE'S", "LIMITS (P14)"],
        0xFF20_2040,
        2,
    );
    let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
        log!("  LIM: no renderer");
        return;
    };
    let _ = r.set_background(Some(GREY));
    let Some(one) = tex16(g, 3, format::RGB565, 1, &pattern565) else {
        log!("  LIM: no texture memory");
        let _ = r.release(&mut g.pool);
        return;
    };
    let uv = [(0, 0), (UV_ONE, 0), (UV_ONE, UV_ONE), (0, UV_ONE)];

    // Objects a frame: the background alone, then K quads.
    let t = g.full(&mut r, T16, 3);
    report("LIM background alone (2400 tiles)", &t, TILES);
    let base = t.median();
    for k in [1usize, 8, 16, 32, 48, 63] {
        for textured in [false, true] {
            grid(g, &mut r, k, &|i| {
                if textured {
                    Paint::Texture {
                        tex: one,
                        uv,
                        tint: WHITE,
                    }
                } else {
                    Paint::Flat(0xFF00_0000 | (i as u32 * 0x0004_0812))
                }
            });
            let t = g.full(&mut r, T16, 3);
            let mut what = heapless_name();
            let _ = write!(
                what,
                "LIM {k} {} 64 x 64 quads",
                if textured { "textured" } else { "flat" }
            );
            report(what.as_str(), &t, TILES);
            log!(
                "    {} us over the background alone, {} ns a quad",
                t.median().saturating_sub(base),
                u64::from(t.median().saturating_sub(base)) * 1000 / k as u64
            );
            rt::check_in();
        }
    }
    clear(g, &mut r);

    // Layers a tile: K full-screen translucent flat layers.
    for k in [8usize, 16, 32, 63] {
        for slot in 1..rp_mbx3d_render::MAX_SLOTS {
            let d = (slot <= k).then(|| {
                Draw::rect(
                    0,
                    0,
                    480,
                    640,
                    Paint::Flat(0x2000_0000 | (slot as u32 * 0x0003_0507)),
                )
                .over()
            });
            g.set(&mut r, slot, d);
        }
        let t = g.full(&mut r, T16, 3);
        let mut what = heapless_name();
        let _ = write!(what, "LIM {k} translucent full-screen layers");
        report(what.as_str(), &t, TILES);
        log!(
            "    {} ns a tile a layer over the background",
            u64::from(t.median().saturating_sub(base)) * 1000 / (u64::from(TILES) * k as u64)
        );
        rt::check_in();
    }
    clear(g, &mut r);

    // Textures a frame: 32 quads on one texture, then on 32.
    let mut own: [Option<Texture>; 32] = [None; 32];
    for (i, t) in own.iter_mut().enumerate() {
        *t = tex16(g, 3, format::RGB565, 1, &|x, y| {
            pattern565(x + 3 * i as u32, y + 5 * i as u32)
        });
    }
    if own.iter().all(Option::is_some) {
        for shared in [true, false] {
            grid(g, &mut r, 32, &|i| Paint::Texture {
                tex: if shared { one } else { own[i].unwrap_or(one) },
                uv,
                tint: WHITE,
            });
            let t = g.full(&mut r, T16, 3);
            report(
                if shared {
                    "LIM 32 quads, one texture"
                } else {
                    "LIM 32 quads, a texture each"
                },
                &t,
                TILES,
            );
            rt::check_in();
        }
    } else {
        log!("  LIM: no memory for 32 textures: skipped");
    }
    for t in own.into_iter().flatten() {
        let _ = rp_mbx3d_render::release(&mut g.pool, t);
    }
    clear(g, &mut r);

    // The texture cache: a whole texture of side 64-1024 over the screen.
    for code in 3u8..=7 {
        let side = 8u32 << code;
        let Some(t) = tex16(g, code, format::RGB565, 1, &pattern565) else {
            log!("  LIM: no memory for a {side} texture");
            continue;
        };
        g.set(
            &mut r,
            1,
            Some(Draw::rect(
                0,
                0,
                480,
                640,
                Paint::Texture {
                    tex: t,
                    uv,
                    tint: WHITE,
                },
            )),
        );
        let times = g.full(&mut r, T16, 3);
        let mut what = heapless_name();
        let _ = write!(
            what,
            "LIM a {side} x {side} texture over the screen ({}.{:02} texels a pixel across)",
            side / 480,
            side % 480 * 100 / 480
        );
        report(what.as_str(), &times, TILES);
        g.set(&mut r, 1, None);
        let _ = rp_mbx3d_render::release(&mut g.pool, t);
        rt::check_in();
    }

    let _ = rp_mbx3d_render::release(&mut g.pool, one);
    let _ = r.release(&mut g.pool);
    log!("LIM: done");
}
