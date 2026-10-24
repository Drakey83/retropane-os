//! The benchmark suite, shared by two images:
//!
//! - `rp-bench` runs it in the emulator (deterministic `-icount`) and prints
//!   each result on the serial port for `cargo xtask bench`.
//! - `rp-probe` runs it on the real X51v and shows the results on screen.
//!
//! Results go to a [`Sink`]: a name, OS-timer ticks (3.25 MHz) and an
//! iteration count. Benchmarks come in comparison pairs where possible
//! (library vs naive, SRAM vs SDRAM, TLB hit vs miss) so they point at what to
//! improve.

#![cfg_attr(target_os = "none", no_std)]
#![forbid(unsafe_code)]

/// Where results go.
pub trait Sink {
    /// One benchmark result.
    fn result(&mut self, name: &'static str, ticks: u32, iters: u32);
    /// A self-check result (`ok` = passed), with a short detail if it failed.
    fn check(&mut self, name: &'static str, ok: bool, detail: &str);
    /// A count of events (not a time: the performance monitor's, C8).
    fn count(&mut self, name: &'static str, n: u32) {
        let _ = (name, n);
    }
}

/// Hops per pointer-chase run (each touches a new cache line).
pub const CHASE_HOPS: u32 = 4096;
/// Sections the TLB benchmark touches when everything fits the 32-entry
/// D-TLB, and when it can't.
pub const TLB_FIT: u32 = 16;
/// See [`TLB_FIT`].
pub const TLB_SPILL: u32 = 60;
/// Bytes `layer` needs: a 480 x 640 ARGB8888 layer and a 64 x 64 glyph.
pub const LAYER_WORK: usize = 480 * 640 * 4 + 64 * 64;

#[cfg(target_os = "none")]
mod target {
    use super::{CHASE_HOPS, Sink, TLB_FIT, TLB_SPILL};
    use core::hint::black_box;

    use rp_drv_mbx_display::{Plane, fill_rect_rgb565, pattern};
    use rp_drv_sdcard::{BLOCK, SdCard, SdHost};
    use rp_hal_pxa27x::rt;
    use rp_kernel_mm::frames::Frames;

    /// Times benchmarks with the OS timer.
    pub struct Runner<'a> {
        sink: &'a mut dyn Sink,
    }

    impl<'a> Runner<'a> {
        /// A runner reporting to `sink`.
        pub fn new(sink: &'a mut dyn Sink) -> Self {
            Self { sink }
        }

        /// Times `iters` runs of `f` and reports the result.
        ///
        /// `f` is called through `dyn` and this is never inlined: with a
        /// generic closure and a constant `iters`, LLVM unrolled whole
        /// benchmarks (180 KB), bloating the image and measuring code nothing
        /// else would run (ADR-0005 rule 8).
        #[inline(never)]
        pub fn run(&mut self, name: &'static str, iters: u32, f: &mut dyn FnMut()) {
            let iters = black_box(iters);
            f(); // warm caches, TLBs and branch predictors
            let start = rt::ticks();
            for _ in 0..iters {
                f();
            }
            self.sink
                .result(name, rt::ticks().wrapping_sub(start), iters);
            // Outside the timing: the starter's run limit (30 s under the
            // loader) holds a whole image, and the suite's runs together
            // outgrew it (rp-probe, 2026-10-22: RunLimit after "done").
            rt::check_in();
        }

        /// Reports a result measured by the caller (ticks for `iters`).
        pub fn result(&mut self, name: &'static str, ticks: u32, iters: u32) {
            self.sink.result(name, ticks, iters);
        }

        /// Reports a self-check.
        pub fn check(&mut self, name: &'static str, ok: bool, detail: &str) {
            self.sink.check(name, ok, detail);
        }

        /// Reports a count of events.
        pub fn count(&mut self, name: &'static str, n: u32) {
            self.sink.count(name, n);
        }
    }

    /// Timer read cost (the floor under every other number).
    pub fn timer(r: &mut Runner) {
        r.run("timer_read", 1000, &mut || {
            black_box(rt::ticks());
        });
    }

    /// The kernel's frame allocator (`rp-kernel-mm`). A 4 MiB pool lives on
    /// the stack only here (5 KiB), keeping images' zeroed RAM in budget.
    #[inline(never)]
    pub fn frames(r: &mut Runner) {
        let mut pool = Frames::<1024>::new();
        let _ = pool.add_range(0, 1024);
        // Worst case: one page out of a whole 4 MiB block splits ten times,
        // and freeing it merges ten times.
        r.run("frames_alloc_free_split", 1000, &mut || {
            if let Some(f) = pool.alloc(black_box(0)) {
                let _ = pool.free(f);
            }
        });
        // Typical use: many pages out, then all back.
        let mut got = [0u16; 1024];
        r.run("frames_fill_drain_1024", 10, &mut || {
            for g in got.iter_mut() {
                *g = pool.alloc(0).map_or(0, |f| f as u16);
            }
            for &g in got.iter() {
                let _ = pool.free(usize::from(g));
            }
        });
    }

    /// `memcpy`/`memset` against a naive loop, plus an exhaustive on-target
    /// correctness check of the memcpy family. `scratch` needs 128 KiB.
    pub fn memory(r: &mut Runner, scratch: &mut [u8]) {
        let check = check_mem(&mut scratch[..1024]);
        match check {
            Ok(()) => r.check("mem", true, ""),
            Err(what) => r.check("mem", false, what),
        }
        let (a, rest) = scratch.split_at_mut(64 * 1024);
        let b = &mut rest[..64 * 1024];
        for (i, v) in a.iter_mut().enumerate() {
            *v = i as u8;
        }
        r.run("memcpy_64k", 20, &mut || {
            b.copy_from_slice(a);
            black_box(&b);
        });
        r.run("memcpy_naive_64k", 20, &mut || naive_copy(b, a));
        r.run("memset_64k", 20, &mut || {
            b.fill(black_box(0x5A));
            black_box(&b);
        });
    }

    /// Pixel paths (item 52, `rp-gfx`): composing in RAM, the glass tint,
    /// an alpha icon, photo conversion, and copying to the framebuffer, whole
    /// screen and a clock-sized dirty rectangle. `work` needs 640 KiB of RAM
    /// (one 480 x 640 surface plus a little). Pairs: the one-multiply blend
    /// vs the per-channel reference; a full flush vs a dirty one.
    pub fn gfx(r: &mut Runner, work: &mut [u8], fb: Option<(&mut [u8], &Plane)>) {
        use rp_gfx::dirty::{self, Dirty};
        use rp_gfx::{AlphaImage, Gradient, Rect, Surface, Tint, blend_reference, rgb};

        const W: u32 = 480;
        const H: u32 = 640;
        let (surf, rest) = work.split_at_mut((W * H * 2) as usize);
        let (Some(px), true) = (
            rp_hal_mem::bytes_as_pixels_mut(surf),
            rest.len() >= 64 * 64 * 3 + 480 * 3,
        ) else {
            r.check("gfx", false, "work area too small or misaligned");
            return;
        };
        let (icon_bytes, rest) = rest.split_at_mut(64 * 64 * 2);
        let (alpha, rest) = rest.split_at_mut(64 * 64);
        let photo = &mut rest[..480 * 3];
        let Some(icon) = rp_hal_mem::bytes_as_pixels_mut(icon_bytes) else {
            r.check("gfx", false, "icon misaligned");
            return;
        };
        for (k, (p, a)) in icon.iter_mut().zip(alpha.iter_mut()).enumerate() {
            *p = (k as u16).wrapping_mul(2654);
            // A round icon: solid inside, soft edge, clear corners.
            let (x, y) = ((k % 64) as i32 - 32, (k / 64) as i32 - 32);
            let d = x * x + y * y;
            *a = if d < 784 {
                255
            } else if d < 1024 {
                128
            } else {
                0
            };
        }
        for (k, b) in photo.iter_mut().enumerate() {
            *b = (k * 7) as u8;
        }
        // The per-channel blend a straightforward version would write, over
        // the whole screen: the "before" for `gfx_blend_fill_vga`.
        let a5 = rp_gfx::alpha5(160);
        r.run("gfx_blend_reference_vga", 2, &mut || {
            for p in px.iter_mut() {
                *p = blend_reference(black_box(0x4208), *p, a5);
            }
        });
        let Some(mut s) = Surface::new(px, W, H, W as usize) else {
            r.check("gfx", false, "surface");
            return;
        };
        let screen = s.bounds();
        let mut g = Gradient::new();
        g.set(
            W,
            Tint {
                color: rgb(6, 8, 11),
                alpha: 190,
            },
            Tint {
                color: rgb(78, 86, 92),
                alpha: 107,
            },
        );

        r.run("gfx_fill_vga", 3, &mut || s.fill(black_box(screen), 0x2945));
        r.run("gfx_glass_vga", 2, &mut || s.tint(black_box(screen), &g));
        r.run("gfx_blend_fill_vga", 2, &mut || {
            s.blend_fill(black_box(screen), 0x4208, 160);
        });
        let img = AlphaImage {
            px: icon,
            alpha,
            w: 64,
            h: 64,
        };
        r.run("gfx_icon_64", 20, &mut || {
            s.draw_alpha(&img, black_box(200), 100)
        });
        r.run("gfx_photo_row_480", 20, &mut || {
            s.put_rgb888_row(0, black_box(7), photo)
        });

        // Self-check: the fast blends against the reference, pixel by pixel
        // on a band, on the target's own arithmetic.
        let mut ok = true;
        s.fill(screen, 0);
        for x in 0..W {
            s.fill(Rect::new(x, 0, 1, 4), (x as u16).wrapping_mul(977));
        }
        let before: [u16; 64] = core::array::from_fn(|k| s.get(k as u32 * 7, 2).unwrap_or(0));
        s.blend_fill(Rect::new(0, 0, W, 4), 0x7BEF, 200);
        for (k, &b) in before.iter().enumerate() {
            let want = blend_reference(0x7BEF, b, rp_gfx::alpha5(200));
            ok &= s.get(k as u32 * 7, 2) == Some(want);
        }

        if let Some((fb, plane)) = fb {
            let stride = plane.stride;
            r.run("gfx_flush_vga", 3, &mut || {
                let _ = dirty::flush(&s.view(), fb, stride, &[black_box(screen)]);
            });
            let mut d = Dirty::<8>::new(screen);
            d.add(Rect::new(371, 9, 101, 24)); // the title bar's clock
            r.run("gfx_flush_clock", 50, &mut || {
                let _ = dirty::flush(&s.view(), fb, stride, black_box(d.rects()));
            });
            // Only the dirty words were written: a marked pixel just outside
            // stays, one inside matches the surface.
            let at = |x: usize, y: usize| y * stride + x * 2;
            let (out_x, in_x) = (368usize, 400usize);
            fb[at(out_x, 20)..at(out_x, 20) + 2].copy_from_slice(&0xBEEFu16.to_le_bytes());
            s.fill(Rect::new(370, 9, 102, 24), 0x1234);
            let _ = dirty::flush(&s.view(), fb, stride, d.rects());
            let px_at = |fb: &[u8], x| u16::from_le_bytes([fb[at(x, 20)], fb[at(x, 20) + 1]]);
            ok &= px_at(fb, out_x) == 0xBEEF && px_at(fb, in_x) == 0x1234;
        }
        r.check(
            "gfx",
            ok,
            if ok {
                ""
            } else {
                "fast path differs from the reference or flush overreached"
            },
        );
    }

    /// The interface layer (item 60, `rp_gfx::layer`): glass as a gradient
    /// fill, a soft-edged glyph over glass (the "over" rule's slow case),
    /// and flushes to an ARGB4444 graphics plane with the dither, whole
    /// screen and clock-sized, against the per-pixel reference conversion.
    /// `work` needs [`LAYER_WORK`] bytes, word aligned; `plane` (the 2700G's
    /// memory, off screen) needs a 480 x 640 ARGB8888 plane's worth.
    pub fn layer(r: &mut Runner, work: &mut [u8], plane: Option<&mut [u8]>) {
        use rp_gfx::Rect;
        use rp_gfx::dirty::Dirty;
        use rp_gfx::layer::{
            self, Layer, Mask, PlaneFormat, over, over_reference, to_4444_dithered,
        };

        const W: u32 = 480;
        const H: u32 = 640;
        let (lw, rest) = work.split_at_mut(super::LAYER_WORK - 64 * 64);
        let cover = &mut rest[..64 * 64];
        let Some(mut l) = Layer::in_bytes(lw, W, H) else {
            r.check("layer", false, "work area too small or misaligned");
            return;
        };
        // A round glyph: solid inside, a soft 4-pixel edge.
        for (k, c) in cover.iter_mut().enumerate() {
            let (x, y) = ((k % 64) as i32 - 32, (k / 64) as i32 - 32);
            let d = x * x + y * y;
            *c = if d < 28 * 28 {
                255
            } else if d < 1024 {
                ((1024 - d) * 255 / (1024 - 784)) as u8
            } else {
                0
            };
        }
        let screen = l.bounds();
        let mut cols = [0u32; W as usize];
        for (k, c) in cols.iter_mut().enumerate() {
            *c = (64 + k as u32 * 96 / W) << 24 | 0x00F0_F8FF;
        }
        r.run("layer_glass_vga", 3, &mut || {
            l.fill_columns(black_box(screen), &cols);
        });
        let mask = Mask {
            cover,
            w: 64,
            h: 64,
        };
        r.run("layer_glyph_over_glass_64", 20, &mut || {
            l.draw_mask(&mask, black_box(0xFF14_1E32), 200, 100);
        });

        // Self-check: the short paths of "over" against the reference on
        // the target's own arithmetic.
        let mut ok = true;
        let mut x = 0x1234_5678u32;
        for _ in 0..2000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let d = x.rotate_left(11);
            ok &= over(x, d) == over_reference(x, d);
            ok &= over(x | 0xFF00_0000, d) == over_reference(x | 0xFF00_0000, d);
            ok &= over(x, d | 0xFF00_0000) == over_reference(x, d | 0xFF00_0000);
        }

        if let Some(plane) = plane {
            let stride = W as usize * 2;
            let Some(fb) = plane.get_mut(..stride * H as usize) else {
                r.check("layer", false, "plane too small");
                return;
            };
            // The "before": converting pixel by pixel with the reference.
            let v = l.view();
            r.run("layer_flush4444_reference_vga", 1, &mut || {
                for y in 0..H {
                    for (x, &c) in v.row(y).iter().enumerate() {
                        let at = y as usize * stride + x * 2;
                        let p = to_4444_dithered(c, x, y as usize).to_le_bytes();
                        fb[at..at + 2].copy_from_slice(&p);
                    }
                }
            });
            // The reference's bytes at a band of sample points, for the
            // fast flush to match.
            let probe: [(u32, u32); 6] = [
                (0, 0),
                (201, 101),
                (231, 131),
                (479, 639),
                (3, 2),
                (250, 140),
            ];
            let at = |x: u32, y: u32| y as usize * stride + x as usize * 2;
            let want: [[u8; 2]; 6] = core::array::from_fn(|k| {
                let (x, y) = probe[k];
                [fb[at(x, y)], fb[at(x, y) + 1]]
            });
            fb.fill(0);
            let f = PlaneFormat::Argb4444Dithered;
            r.run("layer_flush4444_vga", 3, &mut || {
                let _ = layer::flush(&v, f, fb, stride, &[black_box(screen)]);
            });
            for (k, &(x, y)) in probe.iter().enumerate() {
                ok &= fb[at(x, y)..at(x, y) + 2] == want[k];
            }
            let mut d = Dirty::<8>::new(screen);
            d.add(Rect::new(371, 9, 101, 24)); // the title bar's clock
            r.run("layer_flush4444_clock", 50, &mut || {
                let _ = layer::flush(&v, f, fb, stride, black_box(d.rects()));
            });
            // Only the dirty words were written.
            let (out_x, in_x) = (368u32, 400u32);
            fb[at(out_x, 20)..at(out_x, 20) + 2].copy_from_slice(&[0xEF, 0xBE]);
            fb[at(in_x, 20)..at(in_x, 20) + 2].copy_from_slice(&[0xEF, 0xBE]);
            let _ = layer::flush(&v, f, fb, stride, d.rects());
            ok &= fb[at(out_x, 20)..at(out_x, 20) + 2] == [0xEF, 0xBE];
            ok &= fb[at(in_x, 20)..at(in_x, 20) + 2]
                == to_4444_dithered(v.get(in_x, 20).unwrap_or(0), in_x as usize, 20).to_le_bytes();
            if let Some(fb8) = plane.get_mut(..W as usize * 4 * H as usize) {
                r.run("layer_flush8888_vga", 3, &mut || {
                    let _ = layer::flush(
                        &v,
                        PlaneFormat::Argb8888,
                        fb8,
                        W as usize * 4,
                        &[black_box(screen)],
                    );
                });
            }
        }
        r.check(
            "layer",
            ok,
            if ok {
                ""
            } else {
                "fast path differs from the reference or flush overreached"
            },
        );
    }

    /// Full-screen fills on the 2700G's memory.
    pub fn framebuffer(r: &mut Runner, fb: &mut [u8], plane: &Plane) {
        r.run("fb_pattern_vga", 3, &mut || {
            pattern::fill_rgb565(fb, plane.width.into(), plane.height.into(), plane.stride);
        });
        r.run("fb_solid_fill_vga", 3, &mut || {
            fill_rect_rgb565(
                fb,
                plane.stride,
                0,
                0,
                plane.width.into(),
                plane.height.into(),
                black_box(0x001F),
            );
        });
    }

    /// Cache-line aligned, so a DMA engine accepts it.
    #[repr(C, align(32))]
    struct Lines<const N: usize>([u8; N]);

    /// SD reads: one block per command, four per command (CMD18), where
    /// the card's per-command start-up cost is paid once, then the bulk
    /// rows ([`sd_bulk`]) into `big` (cache-line aligned, off the stack).
    pub fn sd<H: SdHost>(r: &mut Runner, card: &mut SdCard<H>, big: &mut [u8]) {
        let mut block = [0u8; BLOCK];
        let mut lba = 0;
        r.run("sd_read_block", 64, &mut || {
            let _ = card.read(lba, &mut block);
            lba = (lba + 1) % 64;
        });
        let mut four = Lines([0u8; 4 * BLOCK]);
        let mut lba = 0;
        r.run("sd_read_4_blocks", 16, &mut || {
            let _ = card.read_many(lba, &mut four.0);
            lba = (lba + 4) % 64;
        });
        sd_bulk(
            r,
            card,
            big,
            ["sd_read_64_blocks", "sd_read_256_blocks"],
            "sd_multi_block_matches",
        );
    }

    /// SD bulk reads (item 58): 64 blocks per command (32 KiB, a cluster's
    /// worth) and 256 (128 KiB, the DMA engine's longest chain), where
    /// moving the data dominates; named `names` so a caller can run them
    /// once per transfer mode (the probe: the FIFO, then DMA). Then `check`:
    /// the multi-block path must read what single-block reads (programmed
    /// I/O) do. Rows that don't fit in `big` are skipped.
    pub fn sd_bulk<H: SdHost>(
        r: &mut Runner,
        card: &mut SdCard<H>,
        big: &mut [u8],
        names: [&'static str; 2],
        check: &'static str,
    ) {
        let mut block = [0u8; BLOCK];
        for (name, blocks, rounds) in [(names[0], 64, 4), (names[1], 256, 2)] {
            if let Some(many) = big.get_mut(..blocks * BLOCK) {
                r.run(name, rounds, &mut || {
                    let _ = card.read_many(0, many);
                });
            }
        }
        if let Some(many) = big.get_mut(..64 * BLOCK) {
            let same = card.read_many(0, many).is_ok()
                && many.chunks_exact(BLOCK).enumerate().all(|(lba, want)| {
                    card.read(lba as u32, &mut block).is_ok() && block[..] == *want
                });
            r.check(check, same, "");
        }
    }

    /// The mini data cache (the CPU gate's V1 and C5): `backing` (1 MiB,
    /// 1 MiB aligned) mapped a second time, once as ordinary sections and
    /// once into the mini data cache, line walks of 512 bytes to 8 KiB
    /// through each (where the mini cache stops holding a walk: Intel says
    /// 2 KiB), then the streaming test: a 24 KiB hot set in `hot` re-walked
    /// after 1 MiB streams through the window; with the stream in the mini
    /// cache the hot set should survive in the main one. Each window is
    /// checked to be `backing`'s own frames.
    pub fn mini_cache(r: &mut Runner, backing: &mut [u8], hot: &mut [u8]) {
        use rp_hal_pxa27x::mmu::Pages;
        const WALKS: [(usize, &str, &str); 6] = [
            (512, "walk_normal_512", "walk_mini_512"),
            (1 << 10, "walk_normal_1k", "walk_mini_1k"),
            (2 << 10, "walk_normal_2k", "walk_mini_2k"),
            (3 << 10, "walk_normal_3k", "walk_mini_3k"),
            (4 << 10, "walk_normal_4k", "walk_mini_4k"),
            (8 << 10, "walk_normal_8k", "walk_mini_8k"),
        ];
        const HOT: usize = 24 << 10;
        let Some(hot) = hot.get_mut(..HOT) else {
            r.check("mini_cache", false, "hot buffer too small");
            return;
        };
        for (i, b) in hot.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut all_same = true;
        for (k, pages) in [Pages::Sections, Pages::MiniSections]
            .into_iter()
            .enumerate()
        {
            let mut l2 = [0u8; 0];
            let stream_name = ["hot_after_stream_normal", "hot_after_stream_mini"][k];
            let mapped =
                rp_hal_pxa27x::diag::with_window(0xC000_0000, backing, pages, &mut l2, &mut |w| {
                    for (i, b) in w.iter_mut().enumerate() {
                        *b = (i / 32) as u8 ^ k as u8;
                    }
                    for &(size, normal, mini) in &WALKS {
                        let area = &w[..size];
                        let iters = ((1 << 20) / size) as u32;
                        r.run([normal, mini][k], iters, &mut || {
                            let mut sum = 0u32;
                            for line in area.chunks_exact(32) {
                                sum = sum.wrapping_add(u32::from(black_box(line)[0]));
                            }
                            black_box(sum);
                        });
                    }
                    // The hot set warmed, 1 MiB streamed through the window,
                    // then the hot set walked once, timed: 16 times.
                    let walk = |h: &[u8]| {
                        let mut sum = 0u32;
                        for line in h.chunks_exact(32) {
                            sum = sum.wrapping_add(u32::from(black_box(line)[0]));
                        }
                        black_box(sum);
                    };
                    let mut ticks = 0u32;
                    for _ in 0..16 {
                        walk(hot);
                        walk(hot);
                        walk(w);
                        let t0 = rt::ticks();
                        walk(hot);
                        ticks = ticks.wrapping_add(rt::ticks().wrapping_sub(t0));
                        rt::check_in();
                    }
                    r.result(stream_name, ticks, 16);
                });
            if !mapped {
                r.check(stream_name, false, "not mapped");
                all_same = false;
                continue;
            }
            // Written through the window, read back at `backing`'s address.
            let same = backing
                .iter()
                .enumerate()
                .step_by(97)
                .all(|(i, &b)| b == (i / 32) as u8 ^ k as u8);
            if !same {
                r.check(stream_name, false, "other frames");
                all_same = false;
            }
        }
        r.check("mini_mapping", all_same, "");
    }

    /// The data cache's real size (the performance gate's CPU lines V1 and
    /// C5): the first byte of every 32-byte line of a buffer read, again
    /// and again, for buffers of 4 KiB to 256 KiB. While the buffer fits the
    /// D-cache every read hits; past it each misses. Per size the result is
    /// one pass (`size / 32` reads), so ns a line = ns a pass / lines.
    /// `buf` needs 256 KiB.
    pub fn cache_walk(r: &mut Runner, buf: &mut [u8]) {
        const SIZES: [(usize, &str); 10] = [
            (4 << 10, "cache_walk_4k"),
            (8 << 10, "cache_walk_8k"),
            (16 << 10, "cache_walk_16k"),
            (24 << 10, "cache_walk_24k"),
            (28 << 10, "cache_walk_28k"),
            (32 << 10, "cache_walk_32k"),
            (40 << 10, "cache_walk_40k"),
            (48 << 10, "cache_walk_48k"),
            (64 << 10, "cache_walk_64k"),
            (256 << 10, "cache_walk_256k"),
        ];
        for (i, b) in buf.iter_mut().take(256 << 10).enumerate() {
            *b = i as u8;
        }
        for (size, name) in SIZES {
            let Some(area) = buf.get(..size) else {
                r.check(name, false, "buffer too small");
                continue;
            };
            let iters = ((1 << 20) / size) as u32;
            r.run(name, iters, &mut || {
                let mut sum = 0u32;
                for line in area.chunks_exact(32) {
                    sum = sum.wrapping_add(u32::from(black_box(line)[0]));
                }
                black_box(sum);
            });
        }
    }

    /// `memcpy` by size (the performance gate's CPU line C7): 64 bytes to
    /// 1 MiB, each repeated to move about 1 MiB a run (the call through
    /// `dyn` is in each repeat: it shows at the small sizes). `buf` needs
    /// 2 MiB.
    pub fn copy_sweep(r: &mut Runner, buf: &mut [u8]) {
        const SIZES: [(usize, &str); 8] = [
            (64, "copy_64b"),
            (256, "copy_256b"),
            (1 << 10, "copy_1k"),
            (4 << 10, "copy_4k"),
            (16 << 10, "copy_16k"),
            (64 << 10, "copy_64k"),
            (256 << 10, "copy_256k"),
            (1 << 20, "copy_1m"),
        ];
        if buf.len() < 2 << 20 {
            r.check("copy_sweep", false, "buffer too small");
            return;
        }
        let (a, rest) = buf.split_at_mut(1 << 20);
        let b = &mut rest[..1 << 20];
        for (i, v) in a.iter_mut().enumerate() {
            *v = i as u8;
        }
        for (size, name) in SIZES {
            let iters = ((1 << 20) / size) as u32;
            let (src, dst) = (&a[..size], &mut b[..size]);
            r.run(name, iters, &mut || {
                dst.copy_from_slice(black_box(src));
                black_box(&dst);
            });
        }
    }

    /// The bulk copy's prefetch distance (the performance gate's CPU line
    /// C7): 256 KiB copied from memory (past the D-cache) with the `pld`
    /// none, 32, 64 (today's `memcpy`), 96, 128, 192 and 256 bytes ahead.
    /// `buf` needs 512 KiB, word-aligned.
    pub fn copy_pld(r: &mut Runner, buf: &mut [u8]) {
        let Some(buf) = rp_hal_mem::bytes_as_words_mut(buf) else {
            r.check("copy_pld", false, "not word-aligned");
            return;
        };
        if buf.len() < 128 * 1024 {
            r.check("copy_pld", false, "buffer too small");
            return;
        }
        let (a, rest) = buf.split_at_mut(64 * 1024);
        let b = &mut rest[..64 * 1024];
        for (i, v) in a.iter_mut().enumerate() {
            *v = i as u32;
        }
        let a = &*a;
        r.run("copy_pld_none_256k", 4, &mut || {
            rp_hal_mem::copy_words_pld::<0>(b, a)
        });
        r.run("copy_pld_32_256k", 4, &mut || {
            rp_hal_mem::copy_words_pld::<32>(b, a)
        });
        r.run("copy_pld_64_256k", 4, &mut || {
            rp_hal_mem::copy_words_pld::<64>(b, a)
        });
        r.run("copy_pld_96_256k", 4, &mut || {
            rp_hal_mem::copy_words_pld::<96>(b, a)
        });
        r.run("copy_pld_128_256k", 4, &mut || {
            rp_hal_mem::copy_words_pld::<128>(b, a)
        });
        r.run("copy_pld_192_256k", 4, &mut || {
            rp_hal_mem::copy_words_pld::<192>(b, a)
        });
        r.run("copy_pld_256_256k", 4, &mut || {
            rp_hal_mem::copy_words_pld::<256>(b, a)
        });
        // Each variant copies right, not just fast.
        let mut right = true;
        for f in [
            rp_hal_mem::copy_words_pld::<0> as fn(&mut [u32], &[u32]),
            rp_hal_mem::copy_words_pld::<32>,
            rp_hal_mem::copy_words_pld::<64>,
            rp_hal_mem::copy_words_pld::<96>,
            rp_hal_mem::copy_words_pld::<128>,
            rp_hal_mem::copy_words_pld::<192>,
            rp_hal_mem::copy_words_pld::<256>,
        ] {
            b.fill(0);
            f(b, a);
            right &= b == a;
        }
        r.check("copy_pld", right, "");
    }

    /// The media unit, iWMMXt (the performance gate's CPU line C6): its ID
    /// register, then each routine against plain ARM code on the same data,
    /// timed and checked equal: 64-bit copies from memory (256 KiB) against
    /// `memcpy`, a saturating 16-bit audio mix and a rounded 50% pixel
    /// average, each in the cache (8 KiB) and from memory (128 KiB). `buf`
    /// needs 512 KiB, 8-byte aligned. Access is put back as found.
    /// Returns the unit's `wCID` and whether access was on before.
    pub fn wmmx(r: &mut Runner, buf: &mut [u8]) -> Option<(u32, bool)> {
        use rp_hal_mem::{
            words_as_bytes, words_as_bytes_mut, words_as_samples as as_i16,
            words_as_samples_mut as as_i16_mut,
        };
        use rp_hal_pxa27x::iwmmxt;
        let Some(buf) = rp_hal_mem::bytes_as_words_mut(buf) else {
            r.check("wmmx", false, "not word-aligned");
            return None;
        };
        if buf.len() < 128 * 1024 || buf.as_ptr() as usize % 8 != 0 {
            r.check("wmmx", false, "buffer too small or not 8-aligned");
            return None;
        }
        let was = iwmmxt::single_access(true);
        let id = iwmmxt::id();
        let (mut c_ok, mut m_ok, mut a_ok) = (true, true, true);

        // Copies: 256 KiB each way.
        {
            let (a, rest) = buf.split_at_mut(64 * 1024);
            let b = &mut rest[..64 * 1024];
            for (i, v) in a.iter_mut().enumerate() {
                *v = (i as u32).wrapping_mul(0x9E37_79B9);
            }
            let (a8, b8) = (words_as_bytes(a), words_as_bytes_mut(b));
            r.run("wmmx_copy_256k", 4, &mut || {
                let _ = iwmmxt::copy(b8, a8);
            });
            r.run("wmmx_memcpy_256k", 4, &mut || b8.copy_from_slice(a8));
            b8.fill(0);
            c_ok &= iwmmxt::copy(b8, a8) && b8 == a8;
        }

        // Mix and average, in the cache then from memory.
        for (cached, words) in [(true, 2 * 1024usize), (false, 32 * 1024)] {
            let (d, rest) = buf.split_at_mut(words);
            let (a, rest) = rest.split_at_mut(words);
            let b = &mut rest[..words];
            for (i, v) in a.iter_mut().enumerate() {
                *v = (i as u32).wrapping_mul(0x9E37_79B9) ^ 0x8000_7FFF;
            }
            for (i, v) in b.iter_mut().enumerate() {
                *v = (i as u32).wrapping_mul(0x85EB_CA6B).rotate_left(7);
            }
            let (a, b) = (&*a, &*b);
            let (mix_w, mix_p, avg_w, avg_p) = if cached {
                ("wmmx_mix_8k", "plain_mix_8k", "wmmx_avg_8k", "plain_avg_8k")
            } else {
                (
                    "wmmx_mix_128k",
                    "plain_mix_128k",
                    "wmmx_avg_128k",
                    "plain_avg_128k",
                )
            };
            let reps = if cached { 64 } else { 4 };
            // Saturating mix: the destination is the running mix, so each
            // pass starts from `a` again (the copy timed apart as noise in
            // both).
            r.run(mix_w, reps, &mut || {
                let _ = iwmmxt::mix(as_i16_mut(d), as_i16(b));
            });
            r.run(mix_p, reps, &mut || plain_mix(as_i16_mut(d), as_i16(b)));
            d.copy_from_slice(a);
            m_ok &= iwmmxt::mix(as_i16_mut(d), as_i16(b));
            let got: u32 = fnv(d);
            d.copy_from_slice(a);
            plain_mix(as_i16_mut(d), as_i16(b));
            m_ok &= fnv(d) == got;

            r.run(avg_w, reps, &mut || {
                let _ = iwmmxt::avg(d, a, b);
            });
            r.run(avg_p, reps, &mut || plain_avg(d, a, b));
            a_ok &= iwmmxt::avg(d, a, b);
            let got = fnv(d);
            plain_avg(d, a, b);
            a_ok &= fnv(d) == got;
        }
        iwmmxt::single_access(was);
        r.check("wmmx_copy", c_ok, "differs from the source");
        r.check("wmmx_mix", m_ok, "differs from the plain mix");
        r.check("wmmx_avg", a_ok, "differs from the plain average");
        Some((id, was))
    }

    /// The performance monitor's event counters (the gate's C8). Known
    /// answers first, which turn the manual's register layout and event
    /// numbers into facts: a loop of `N` `subs`/`bne` pairs is `2 N`
    /// instructions and `N` branches (a few more for the reads around it);
    /// a line-stride read of 64 KiB, twice the D-cache, misses every line
    /// (2,048) once warm, 16 KiB none. Then what the work costs in events:
    /// a 64 KiB `memcpy` from memory and a 256 KiB pointer chase. `buf`
    /// needs 256 KiB.
    pub fn pmu(r: &mut Runner, buf: &mut [u8]) {
        use rp_hal_pxa27x::pmu::{self, Event};
        const N: u32 = 10_000;
        let s = pmu::start([Event::Instruction, Event::Branch, Event::DcacheMiss]);
        pmu::known_loop(N);
        let c = pmu::read(s);
        let [insns, branches, _] = c.events;
        r.count("pmu_loop_insns", insns);
        r.count("pmu_loop_branches", branches);
        r.check("pmu_insns", (2 * N..2 * N + 64).contains(&insns), "not 2N");
        r.check("pmu_branches", (N..N + 16).contains(&branches), "not N");
        let Some(area) = buf.get_mut(..256 << 10) else {
            r.check("pmu_dmiss", false, "buffer too small");
            return;
        };
        let walk = |a: &[u8]| {
            let mut sum = 0u32;
            for line in a.chunks_exact(32) {
                sum = sum.wrapping_add(u32::from(black_box(line)[0]));
            }
            black_box(sum);
        };
        let mut misses = [0u32; 2];
        for (m, size) in misses.iter_mut().zip([64 << 10, 16 << 10]) {
            walk(&area[..size]);
            let s = pmu::start([Event::DcacheMiss, Event::DcacheAccess, Event::DtlbMiss]);
            walk(&area[..size]);
            *m = pmu::read(s).events[0];
        }
        r.count("pmu_walk64k_dmiss", misses[0]);
        r.count("pmu_walk16k_dmiss", misses[1]);
        r.check(
            "pmu_dmiss",
            (1_800..2_300).contains(&misses[0]) && misses[1] < 64,
            "miss counts off",
        );
        // What the work costs.
        let (src, dst) = area.split_at_mut(128 << 10);
        let s = pmu::start([
            Event::DcacheMiss,
            Event::DcacheWriteback,
            Event::BufferStallCycles,
        ]);
        dst[..64 << 10].copy_from_slice(&src[..64 << 10]);
        let c = pmu::read(s);
        r.count("pmu_copy64k_cycles", c.cycles);
        r.count("pmu_copy64k_dmiss", c.events[0]);
        r.count("pmu_copy64k_writeback", c.events[1]);
        r.count("pmu_copy64k_bufstall", c.events[2]);
    }

    /// The multiply's cost by its multiplier's size: 1,000 dependent
    /// multiplies a run by `k` of 1, 15, 16, 27 and 32 significant bits
    /// (rp-lz4's hash multiplies by a 32-bit constant: ADR-0001 "Measured
    /// again"). ns per 1,000.
    pub fn mul_sizes(r: &mut Runner) {
        for (name, k) in [
            ("mul_chain_k1", 1u32),
            ("mul_chain_k15", 0x4F1B),
            ("mul_chain_k16", 0x9E37),
            ("mul_chain_k27", 0x04F1_BBCD),
            ("mul_chain_k32", 0x9E37_79B1),
        ] {
            let mut x = 0x1234_5679u32;
            r.run(name, 8, &mut || {
                x = rp_hal_pxa27x::cpu::mul_chain(1000, black_box(k), x);
            });
            black_box(x);
        }
    }

    /// The resampler's filter (C6's mixer step): one channel's 48 kHz output
    /// sample, 11 taps a wing, three ways on the same data, a block of 32
    /// frames by two channels each time: the mixer's loop as it is (an
    /// `i32` window, the left wing reversed against its own coefficients),
    /// one plain contiguous dot product (the left coefficients stored
    /// reversed, 16-bit window, padded to 24), and iWMMXt's [`dot`]
    /// (`WMADDS`). Checked equal at every window offset (all four
    /// alignments).
    ///
    /// [`dot`]: rp_hal_pxa27x::iwmmxt::dot
    pub fn wmmx_dot(r: &mut Runner) {
        use rp_hal_pxa27x::iwmmxt;
        const TAPS: usize = 11;
        const PAD: usize = 24;
        const WIN: usize = 128;
        #[repr(align(8))]
        struct Win([i16; WIN + 8]);
        #[repr(align(8))]
        struct Coef([i16; PAD]);
        let mut w16 = Win([0; WIN + 8]);
        let mut w32 = [0i32; WIN];
        let mut seed = 0x1234_5678u32;
        for k in 0..WIN {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            w16.0[k] = (seed >> 16) as i16;
            w32[k] = i32::from(w16.0[k]);
        }
        let (mut cl, mut cr) = ([0i16; 22], [0i16; 22]);
        for k in 0..TAPS {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            cl[k] = ((seed >> 20) as i16) - 2048;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            cr[k] = ((seed >> 20) as i16) - 2048;
        }
        let mut c = Coef([0; PAD]);
        for k in 0..TAPS {
            c.0[k] = cl[TAPS - 1 - k];
            c.0[TAPS + k] = cr[k];
        }
        // The mixer's shape.
        let shaped = |base: usize| -> i32 {
            let lw = &w32[base + 1 - TAPS..=base];
            let rw = &w32[base + 1..=base + TAPS];
            let mut sum = 0i32;
            for ((x, y), (cx, cy)) in lw.iter().rev().zip(rw).zip(cl.iter().zip(&cr)) {
                sum = sum
                    .wrapping_add(x.wrapping_mul(i32::from(*cx)))
                    .wrapping_add(y.wrapping_mul(i32::from(*cy)));
            }
            sum
        };
        let flat = |base: usize| -> i32 {
            let at = base + 1 - TAPS;
            w16.0[at..at + PAD]
                .iter()
                .zip(&c.0)
                .fold(0i32, |s, (x, k)| {
                    s.wrapping_add(i32::from(*x) * i32::from(*k))
                })
        };
        let was = iwmmxt::single_access(true);
        let mut same = true;
        for base in TAPS - 1..WIN - PAD {
            let want = shaped(base);
            same &= flat(base) == want && iwmmxt::dot(&w16.0, base + 1 - TAPS, &c.0) == Some(want);
        }
        // A block: 32 frames by two channels, frames stepping through the
        // window (as at 48 kHz: about 1.09 source samples a frame).
        let bases = |f: usize| TAPS - 1 + (f * 109) / 100;
        r.run("dot48_mixer_shape_block", 200, &mut || {
            let mut s = 0i32;
            for f in 0..64 {
                s = s.wrapping_add(shaped(bases(f / 2)));
            }
            black_box(s);
        });
        r.run("dot48_plain_flat_block", 200, &mut || {
            let mut s = 0i32;
            for f in 0..64 {
                s = s.wrapping_add(flat(bases(f / 2)));
            }
            black_box(s);
        });
        r.run("dot48_wmmx_block", 200, &mut || {
            let mut s = 0i32;
            for f in 0..64 {
                s = s.wrapping_add(iwmmxt::dot(&w16.0, bases(f / 2) + 1 - TAPS, &c.0).unwrap_or(0));
            }
            black_box(s);
        });
        // WALIGNR at every offset against the bytes it should take (offset 0
        // is the low word whole: the emulator's shift by 64 got it wrong).
        let (lo, hi) = (0x0706_0504_0302_0100u64, 0x0F0E_0D0C_0B0A_0908u64);
        let mut aligned = true;
        for n in 0..8u32 {
            let want = if n == 0 {
                lo
            } else {
                lo >> (8 * n) | hi << (64 - 8 * n)
            };
            aligned &= iwmmxt::align(lo, hi, n) == want;
        }
        iwmmxt::single_access(was);
        r.check("wmmx_dot", same, "a filter sum differs between the three");
        r.check("wmmx_align", aligned, "WALIGNR took the wrong bytes");
    }

    /// Plain ARM: `d[i] = d[i] + s[i]` saturating, two samples a word.
    #[inline(never)]
    fn plain_mix(d: &mut [i16], s: &[i16]) {
        for (x, y) in d.iter_mut().zip(s) {
            *x = x.saturating_add(*y);
        }
    }

    /// Plain ARM: the rounded byte-wise average, four bytes a word (the
    /// usual `(a | b) - ((a ^ b) & 0xFE..) / 2`).
    #[inline(never)]
    fn plain_avg(d: &mut [u32], a: &[u32], b: &[u32]) {
        for ((x, p), q) in d.iter_mut().zip(a).zip(b) {
            *x = (p | q) - (((p ^ q) & 0xFEFE_FEFE) >> 1);
        }
    }

    fn fnv(w: &[u32]) -> u32 {
        w.iter()
            .fold(0x811C_9DC5, |h, &v| (h ^ v).wrapping_mul(0x0100_0193))
    }

    /// The DMA controller as a copy engine (the performance gate's CPU line
    /// C11): 128 KiB SDRAM to SDRAM by DMA (32-byte bursts, words) against
    /// `memcpy`, each checked; and how far the CPU gets counting while the
    /// DMA copies, against the same count with nothing else running.
    /// `buf` needs 256 KiB, cache-line aligned.
    pub fn dma_copy(r: &mut Runner, dma: &mut rp_hal_pxa27x::dma::Dma, buf: &mut [u8]) {
        use rp_hal_api::DmaEngine;
        if buf.len() < 256 * 1024 {
            r.check("dma_copy", false, "buffer too small");
            return;
        }
        let (a, rest) = buf.split_at_mut(128 * 1024);
        let b = &mut rest[..128 * 1024];
        for (i, v) in a.iter_mut().enumerate() {
            *v = (i * 7) as u8;
        }
        let mut refused = false;
        r.run("dma_copy_128k", 4, &mut || {
            if !dma.arm_copy(a, b) {
                refused = true;
                return;
            }
            while dma.done().is_none() {}
        });
        b.fill(0);
        let ok = dma.arm_copy(a, b) && {
            while dma.done().is_none() {}
            a == b
        };
        // The same with a pause between the status polls (the SD reads ran
        // 28% faster so, 2026-10-22): does the polling slow this DMA too?
        r.run("dma_copy_128k_paced", 4, &mut || {
            if !dma.arm_copy(a, b) {
                refused = true;
                return;
            }
            while dma.done().is_none() {
                for k in 0..200u32 {
                    black_box(k);
                }
            }
        });
        r.check(
            "dma_copy",
            ok && !refused,
            if refused { "refused" } else { "wrong" },
        );
        r.run("memcpy_128k", 4, &mut || {
            b.copy_from_slice(black_box(&*a));
            black_box(&b);
        });
        // C11's sweep: the burst size and the width field (0 or word; the
        // shipped copy is 32-byte bursts, word), each timed and checked.
        // Every wait is bounded (2^22 polls, far past any copy's time): a
        // shape the controller doesn't finish is aborted and reported, not
        // a frozen probe.
        use rp_hal_api::Width;
        // Spins between polls: polling the controller back to back slows
        // the copy it waits for (as SD's, pxa-mmc's POLL_SPIN), so a faster
        // poll loop made the 8-byte-burst copy 4.7% slower (ADR-0001
        // "Measured again"). Swept on the X51v 2026-10-22 (8-byte bursts,
        // ms a 128 KiB copy): none 8.90, 50 8.50, 200 8.47, 1,000 8.25,
        // 3,000 8.26, 10,000 8.26 (16-byte 4.74 -> 4.54, 32-byte 2.92 ->
        // 2.91): 1,000, where it levels off. `RP_PROBE_DMA_POLL_SPIN`
        // overrides it.
        const POLL_SPIN: u32 = match option_env!("RP_PROBE_DMA_POLL_SPIN") {
            Some(s) => rp_hal_api::parse_u32(s),
            None => 1000,
        };
        let finish = |dma: &mut rp_hal_pxa27x::dma::Dma| -> bool {
            for _ in 0..1u32 << 22 {
                for k in 0..POLL_SPIN {
                    black_box(k);
                }
                if let Some(ok) = dma.done() {
                    return ok;
                }
            }
            dma.abort();
            false
        };
        let mut shaped = true;
        let mut stuck = 0u32;
        for (name, burst, w) in [
            ("dma_copy_128k_b8_word", 8, Some(Width::Word)),
            ("dma_copy_128k_b16_word", 16, Some(Width::Word)),
            // Width 0 (32, 16, 8) timed the same as word on the X51v
            // (2026-10-22, local/rp-probe-dma-shapes.txt): one kept, as
            // Linux sets it, now that the emulator copies it too
            // (rp-v0.30.0; rp-v0.29.0 ran away: docs/hw/emulator.md).
            ("dma_copy_128k_b32_w0", 32, None),
        ] {
            b.fill(0);
            let right = dma.arm_copy_shaped(a, b, burst, w) && finish(dma) && a == b;
            if !right {
                // Not timed: a wrong or stuck shape is the finding.
                stuck += 1;
                shaped = false;
                continue;
            }
            r.run(name, 4, &mut || {
                if dma.arm_copy_shaped(a, b, burst, w) {
                    let _ = finish(dma);
                }
            });
        }
        r.check(
            "dma_shapes",
            shaped && !refused,
            if refused {
                "refused"
            } else {
                "a shape copied wrong or stuck"
            },
        );
        r.result("dma_shapes_wrong", stuck, 1);
        // The CPU's share: units of cached work (1,000 dependent adds)
        // done while the DMA copies (its status read once a unit), then the
        // same number of units with nothing else running; ns a unit each.
        let unit = || {
            let mut x = 0u32;
            for i in 0..1000u32 {
                x = black_box(x.wrapping_add(i));
            }
            black_box(x);
        };
        let mut units = 0u32;
        let t0 = rt::ticks();
        if dma.arm_copy(a, b) {
            loop {
                unit();
                units += 1;
                if dma.done().is_some() {
                    break;
                }
            }
        }
        let during = rt::ticks().wrapping_sub(t0);
        let t1 = rt::ticks();
        for _ in 0..units {
            unit();
        }
        let alone = rt::ticks().wrapping_sub(t1);
        r.result("cpu_unit_during_dma", during, units.max(1));
        r.result("cpu_unit_alone", alone, units.max(1));
    }

    /// The internal SRAM's every word (the performance gate's CPU line V2:
    /// its size and soundness on this unit, not the manual's word): each
    /// word written and read back with 0x55555555, 0xAAAAAAAA, its own
    /// address and its complement, in turn over the whole buffer (so a
    /// word that aliases another, past the real end, shows). `sram` is the
    /// whole SRAM.
    pub fn sram_patterns(r: &mut Runner, sram: &mut [u8]) {
        let Some(words) = rp_hal_mem::bytes_as_words_mut(sram) else {
            r.check("sram_patterns", false, "not aligned");
            return;
        };
        let n = words.len();
        let mut bad = 0usize;
        for pass in 0..4u32 {
            let want = |i: usize| match pass {
                0 => 0x5555_5555,
                1 => 0xAAAA_AAAA,
                2 => i as u32,
                _ => !(i as u32),
            };
            for (i, w) in words.iter_mut().enumerate() {
                *w = want(i);
            }
            bad += words
                .iter()
                .enumerate()
                .filter(|&(i, &w)| black_box(w) != want(i))
                .count();
        }
        r.check(
            "sram_patterns",
            bad == 0 && n == 64 * 1024,
            if bad != 0 {
                "a word wrong"
            } else if n != 64 * 1024 {
                "not 256 KiB"
            } else {
                ""
            },
        );
    }

    /// The internal SRAM against SDRAM (the performance gate's CPU lines V2
    /// and C3): 64 KiB copied within the SRAM, from SDRAM into it, and
    /// filled. `sram` needs 128 KiB, `sdram` 64 KiB.
    pub fn sram_bandwidth(r: &mut Runner, sram: &mut [u8], sdram: &mut [u8]) {
        if sram.len() < 128 << 10 || sdram.len() < 64 << 10 {
            r.check("sram_bandwidth", false, "buffer too small");
            return;
        }
        let (a, rest) = sram.split_at_mut(64 << 10);
        let b = &mut rest[..64 << 10];
        let d = &mut sdram[..64 << 10];
        for (i, v) in a.iter_mut().enumerate() {
            *v = i as u8;
        }
        r.run("memcpy_sram_64k", 20, &mut || {
            b.copy_from_slice(black_box(&*a));
            black_box(&b);
        });
        r.run("memcpy_sdram_to_sram_64k", 20, &mut || {
            b.copy_from_slice(black_box(&*d));
            black_box(&b);
        });
        r.run("memset_sram_64k", 20, &mut || {
            b.fill(black_box(0x5A));
            black_box(&b);
        });
    }

    /// Memory latency by pointer chasing: each hop depends on the previous
    /// load and lands on a new cache line, so once the chain outgrows the
    /// 32 KiB D-cache this is the miss latency of `buf`'s memory. Run on the
    /// internal SRAM and on SDRAM (ADR-0006 rule 1).
    pub fn chase(r: &mut Runner, name: &'static str, buf: &mut [u8]) {
        let lines = (buf.len() / 32).next_power_of_two() / 2; // power of two <= len
        if lines < 2048 {
            r.check(name, false, "buffer too small");
            return;
        }
        // next(i) = (i + step) mod lines, step odd: one cycle through every
        // line, in large jumps the prefetcher can't follow.
        let step = lines * 5 / 8 + 1;
        for i in 0..lines {
            let next = ((i + step) % lines) as u32;
            buf[i * 32..i * 32 + 4].copy_from_slice(&next.to_le_bytes());
        }
        let mut at = 0usize;
        r.run(name, 8, &mut || {
            for _ in 0..CHASE_HOPS {
                let b = &buf[at * 32..at * 32 + 4];
                at = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
            }
            black_box(at);
        });
    }

    /// SHA-256 and HMAC on the target: checked against the published
    /// vectors (the ARM build, not just the host's), then timed on a 4 KiB
    /// page and a 64-byte message (the TLS record and handshake shapes).
    pub fn crypto(r: &mut Runner, page: &mut [u8]) {
        use rp_crypto::{HmacSha256, Sha256, ct_eq};
        // The published answers, computed by this build on this CPU.
        let abc = Sha256::digest(b"abc");
        let want_abc: [u8; 8] = [0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea];
        let tag = HmacSha256::mac(b"Jefe", b"what do ya want for nothing?");
        let want_tag: [u8; 8] = [0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e];
        r.check(
            "crypto",
            ct_eq(&abc[..8], &want_abc) && ct_eq(&tag[..8], &want_tag),
            "",
        );
        let Some(page) = page.get_mut(..4096) else {
            return;
        };
        for (i, b) in page.iter_mut().enumerate() {
            *b = i as u8;
        }
        r.run("sha256_4k", 8, &mut || {
            black_box(Sha256::digest(black_box(&*page)));
        });
        r.run("hmac_sha256_64b", 64, &mut || {
            black_box(HmacSha256::mac(b"key", black_box(&page[..64])));
        });
    }

    /// One row of [`pages`]: the chase's name, its D-TLB miss count's, the
    /// page size, and whether the second-level tables sit in the internal
    /// SRAM.
    pub struct PageRow {
        /// The chase's result name.
        pub name: &'static str,
        /// The D-TLB miss count's name.
        pub tlb: &'static str,
        /// The page-hopping chase's result name ([`far_chase`]).
        pub far: &'static str,
        /// Its D-TLB miss count's name.
        pub far_tlb: &'static str,
        /// How the window is mapped.
        pub pages: rp_hal_pxa27x::mmu::Pages,
        /// The second-level tables in the internal SRAM (else SDRAM).
        pub l2_sram: bool,
    }

    /// The CPU gate's C4 and V7: [`chase`] over `backing` (SDRAM, 1 MiB
    /// aligned, whole MiBs: past the 32-entry D-TLB's reach at every page
    /// size used) mapped a second time at `0xC0000000` as each row says,
    /// then one more pass of 4,096 hops with the D-TLB misses counted.
    /// `l2_sdram` and `l2_sram`: second-level tables, 1 KiB aligned, 1 KiB
    /// a MiB.
    pub fn pages(
        r: &mut Runner,
        backing: &mut [u8],
        l2_sdram: &mut [u8],
        mut l2_sram: Option<&mut [u8]>,
        rows: &[PageRow],
    ) {
        use rp_hal_pxa27x::pmu::{self, Event};
        let mut all_same = true;
        for row in rows {
            let l2 = if row.l2_sram {
                match l2_sram.as_deref_mut() {
                    Some(t) => t,
                    None => {
                        r.check(row.name, false, "no SRAM");
                        continue;
                    }
                }
            } else {
                &mut *l2_sdram
            };
            let mut misses = 0;
            let mapped =
                rp_hal_pxa27x::diag::with_window(0xC000_0000, backing, row.pages, l2, &mut |w| {
                    // First (`chase` then writes over half of its chain,
                    // the half checked below).
                    far_chase(r, row.far, row.far_tlb, w);
                    chase(r, row.name, w);
                    // `chase` left its chain in the window: one more pass,
                    // counted.
                    let s = pmu::start([Event::DtlbMiss, Event::DcacheMiss, Event::Instruction]);
                    let mut at = 0usize;
                    for _ in 0..CHASE_HOPS {
                        let b = &w[at * 32..at * 32 + 4];
                        at = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
                    }
                    black_box(at);
                    misses = pmu::read(s).events[0];
                });
            if !mapped {
                r.check(row.name, false, "not mapped");
                continue;
            }
            r.count(row.tlb, misses);
            // The window must have been `backing`'s own frames: the chain
            // `chase` wrote through it, read back at `backing`'s address
            // (the window was written back as it closed).
            let lines = (backing.len() / 32).next_power_of_two() / 2;
            let step = lines * 5 / 8 + 1;
            let same = (0..lines).all(|i| {
                let b = &backing[i * 32..i * 32 + 4];
                u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == ((i + step) % lines) as u32
            });
            if !same {
                all_same = false;
                r.check(row.name, false, "other frames");
            }
        }
        r.check("pages_mapping", all_same, "");
    }

    /// Lines a [`far_chase`] hop moves: 4,128 bytes, so every hop lands on
    /// a new 4 KiB page and a new cache line (odd, so the chain visits
    /// every line of a power-of-two buffer).
    const FAR_STEP: usize = 129;

    /// The worst case for the D-TLB: [`CHASE_HOPS`] hops, each to the next
    /// 4 KiB page (with 4 KiB pages every hop misses; with 64 KiB pages one
    /// in 16; with sections none), timed, then one pass with the misses
    /// counted. [`chase`] jumps far but stays within a few pages at a time,
    /// so it measures the cache, not the TLB (`rp-probe` pages run 1).
    pub fn far_chase(r: &mut Runner, name: &'static str, tlb: &'static str, buf: &mut [u8]) {
        use rp_hal_pxa27x::pmu::{self, Event};
        let lines = (buf.len() / 32).next_power_of_two() / 2;
        if lines < 2048 {
            r.check(name, false, "buffer too small");
            return;
        }
        for i in 0..lines {
            let next = ((i + FAR_STEP) % lines) as u32;
            buf[i * 32..i * 32 + 4].copy_from_slice(&next.to_le_bytes());
        }
        let hop = |buf: &[u8], at: &mut usize| {
            for _ in 0..CHASE_HOPS {
                let b = &buf[*at * 32..*at * 32 + 4];
                *at = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
            }
            black_box(*at);
        };
        let mut at = 0usize;
        r.run(name, 8, &mut || hop(buf, &mut at));
        let s = pmu::start([Event::DtlbMiss, Event::DcacheMiss, Event::Instruction]);
        hop(buf, &mut at);
        r.count(tlb, pmu::read(s).events[0]);
    }

    /// Pages [`tlb_lock`] hops between: twice the D-TLB's 32 entries.
    const LOCK_PAGES: usize = 64;
    /// Of them, locked in the second row.
    const LOCKED: usize = 16;

    /// The CPU gate's C4, TLB lock-down: in `w` (mapped with 4 KiB pages,
    /// at least [`LOCK_PAGES`] of them), a chain through one line of each
    /// of [`LOCK_PAGES`] pages in turn (the lines on different cache sets,
    /// so all stay in the cache and each hop's cost is the TLB's), timed
    /// with nothing locked (`none`), then with the first [`LOCKED`] pages'
    /// entries locked (`locked`), the D-TLB misses of one pass counted for
    /// each (`none_dtlb`, `locked_dtlb`). Locked, a quarter of the hops
    /// should stop missing. Everything is unlocked before it returns.
    pub fn tlb_lock(
        r: &mut Runner,
        w: &mut [u8],
        [none, locked, none_dtlb, locked_dtlb]: [&'static str; 4],
    ) {
        use rp_hal_pxa27x::pmu::{self, Event};
        if w.len() < LOCK_PAGES * 4096 {
            r.check(none, false, "window too small");
            return;
        }
        let at = |p: usize| p * 4096 + (p % 32) * 32;
        for p in 0..LOCK_PAGES {
            let next = at((p + 1) % LOCK_PAGES) as u32;
            w[at(p)..at(p) + 4].copy_from_slice(&next.to_le_bytes());
        }
        let hop = |w: &[u8], a: &mut usize| {
            for _ in 0..CHASE_HOPS {
                let b = &w[*a..*a + 4];
                *a = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
            }
            black_box(*a);
        };
        for (name, count, lock) in [(none, none_dtlb, false), (locked, locked_dtlb, true)] {
            if lock {
                for p in 0..LOCKED {
                    rp_hal_pxa27x::cpu::lock_dtlb(w.as_ptr() as u32 + at(p) as u32);
                }
            }
            let mut a = 0usize;
            r.run(name, 8, &mut || hop(w, &mut a));
            let s = pmu::start([Event::DtlbMiss, Event::DcacheMiss, Event::Instruction]);
            hop(w, &mut a);
            r.count(count, pmu::read(s).events[0]);
        }
        rp_hal_pxa27x::cpu::unlock_dtlb();
    }

    /// TLB-miss cost: one read per 1 MiB section of SDRAM, over
    /// [`TLB_FIT`] sections (all fit the 32-entry D-TLB) and [`TLB_SPILL`]
    /// (every read misses). The reads stay in cache (each section's address is
    /// offset to its own cache set), so the difference is the page-table walk.
    pub fn tlb(r: &mut Runner, fit: &'static str, spill: &'static str) {
        for (name, n) in [(fit, TLB_FIT), (spill, TLB_SPILL)] {
            // 1024 rounds: at 64 the whole run was ~26 timer ticks, so one
            // tick of granularity read as a 4% change.
            r.run(name, 1024, &mut || {
                for i in 0..n {
                    let addr = 0xA800_0000 + (i << 20) + ((i % 32) << 5);
                    black_box(rt::fault_probe(addr as usize));
                }
            });
        }
    }

    /// The compressor's table as the kernel keeps it (`kernel/mm`'s
    /// `pack`: one table in kernel memory for every page, never cleared,
    /// `rp_lz4::compress_with`), so the compress rows time what ships.
    static LZ4_TABLE: rt::Global<[u16; rp_lz4::TABLE_LEN]> =
        rt::Global::new([0; rp_lz4::TABLE_LEN]);

    /// LZ4 on 4 KiB pages (ADR-0005: compressed RAM and storage), three
    /// kinds: a heap-shaped stress page (pointers and small integers: ~1000
    /// minimum-length matches), text (UI strings and resources: long
    /// matches), and noise (already-compressed data: nothing to find). The
    /// round trip is checked on the CPU for each. Compressed as the kernel
    /// does, on [`LZ4_TABLE`]. `scratch` needs 16 KiB.
    pub fn lz4(r: &mut Runner, scratch: &mut [u8]) {
        let (page, rest) = scratch.split_at_mut(4096);
        let (packed, rest) = rest.split_at_mut(rp_lz4::max_compressed(4096));
        let back = &mut rest[..4096];
        let kinds: [(&'static str, &'static str, &'static str, fn(&mut [u8])); 3] = [
            ("lz4", "lz4_compress_page", "lz4_decompress_page", heap_page),
            (
                "lz4 text",
                "lz4_compress_text",
                "lz4_decompress_text",
                text_page,
            ),
            (
                "lz4 noise",
                "lz4_compress_noise",
                "lz4_decompress_noise",
                noise_page,
            ),
        ];
        for (check, comp, decomp, fill) in kinds {
            fill(page);
            let n = LZ4_TABLE
                .with(|t| rp_lz4::compress_with(t, page, packed))
                .unwrap_or(0);
            let ok =
                n > 0 && rp_lz4::decompress(&packed[..n], back) == Ok(4096) && back == &page[..];
            r.check(check, ok, if ok { "" } else { "round trip" });
            r.run(comp, 16, &mut || {
                black_box(
                    LZ4_TABLE
                        .with(|t| rp_lz4::compress_with(t, black_box(&page[..]), packed))
                        .ok(),
                );
            });
            r.run(decomp, 64, &mut || {
                black_box(rp_lz4::decompress(black_box(&packed[..n]), back).ok());
            });
            if check == "lz4 text" {
                lz4_text_events(r, page, packed);
            }
        }
    }

    /// Where text compress's time goes (ADR-0001 "Why, and what was
    /// fixed": it lost 4-6% with frame pointers off and the listing didn't
    /// say why): 16 compressions of the text page counted twice, six events
    /// and the cycles ([Intel] event numbers, C8's checks).
    fn lz4_text_events(r: &mut Runner, page: &[u8], packed: &mut [u8]) {
        use rp_hal_pxa27x::pmu::{self, Event};
        // As the rows: the kernel's kept table.
        let once = |packed: &mut [u8]| {
            black_box(
                LZ4_TABLE
                    .with(|t| rp_lz4::compress_with(t, black_box(page), packed))
                    .ok(),
            );
        };
        let mut twice = |events: [Event; 3]| {
            once(packed);
            let s = pmu::start(events);
            for _ in 0..16 {
                once(packed);
            }
            pmu::read(s)
        };
        let a = twice([Event::Instruction, Event::DataStall, Event::IcacheStall]);
        let b = twice([
            Event::BranchMiss,
            Event::DcacheMiss,
            Event::BufferStallCycles,
        ]);
        r.count("lz4t_cycles", a.cycles);
        r.count("lz4t_insns", a.events[0]);
        r.count("lz4t_data_stall", a.events[1]);
        r.count("lz4t_fetch_stall", a.events[2]);
        r.count("lz4t_branch_miss", b.events[0]);
        r.count("lz4t_dcache_miss", b.events[1]);
        r.count("lz4t_buffer_stall", b.events[2]);
        // The same split three ways (the last 2% of text compress): the
        // table's clear alone, then `compress_with` on a table of ours
        // (its clear and the loop); D-cache misses, buffer-stall cycles,
        // data stalls and cycles each.
        let mut table = [0u16; rp_lz4::TABLE_LEN];
        let span = |r: &mut Runner, name: [&'static str; 4], f: &mut dyn FnMut()| {
            f();
            let s = pmu::start([
                Event::DcacheMiss,
                Event::BufferStallCycles,
                Event::DataStall,
            ]);
            for _ in 0..16 {
                f();
            }
            let c = pmu::read(s);
            r.count(name[0], c.events[0]);
            r.count(name[1], c.events[1]);
            r.count(name[2], c.events[2]);
            r.count(name[3], c.cycles);
        };
        span(
            r,
            ["lz4c_dmiss", "lz4c_bstall", "lz4c_dstall", "lz4c_cycles"],
            &mut || black_box(&mut table).fill(0),
        );
        // Read the whole table, then clear it: if reads put its lines in
        // the cache, this clear hits.
        span(
            r,
            ["lz4r_dmiss", "lz4r_bstall", "lz4r_dstall", "lz4r_cycles"],
            &mut || {
                let t = black_box(&mut table);
                let mut sum = 0u32;
                for e in t.iter().step_by(16) {
                    sum = sum.wrapping_add(u32::from(*e));
                }
                black_box(sum);
                t.fill(0);
            },
        );
        span(
            r,
            ["lz4w_dmiss", "lz4w_bstall", "lz4w_dstall", "lz4w_cycles"],
            &mut || {
                black_box(rp_lz4::compress_with(&mut table, black_box(page), packed).ok());
            },
        );
    }

    fn heap_page(page: &mut [u8]) {
        for (i, w) in page.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let i = i as u32;
            let v = if i % 3 == 0 {
                0xA800_0000 + (i % 37) * 16
            } else {
                i % 11
            };
            *w = v.to_le_bytes();
        }
    }

    fn text_page(page: &mut [u8]) {
        const WORDS: [&[u8]; 16] = [
            b"the ",
            b"Settings ",
            b"Today ",
            b"Start ",
            b"menu ",
            b"of ",
            b"Contacts ",
            b"and ",
            b"Calendar ",
            b"to ",
            b"Messaging ",
            b"a ",
            b"Wi-Fi ",
            b"is ",
            b"connected. ",
            b"Programs ",
        ];
        let mut x = 0x1234_5678u32;
        let mut at = 0;
        while at < page.len() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            for &b in WORDS[(x % 16) as usize] {
                if at < page.len() {
                    page[at] = b;
                    at += 1;
                }
            }
        }
    }

    fn noise_page(page: &mut [u8]) {
        let mut x = 0x9E37_79B9u32;
        for b in page.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = x as u8;
        }
    }

    /// Checks the memcpy family on the real CPU for every size 0..=300 and
    /// every alignment, including overlapping moves and comparisons.
    fn check_mem(buf: &mut [u8]) -> Result<(), &'static str> {
        let (src, rest) = buf.split_at_mut(512);
        let dst = &mut rest[..512];
        for (i, v) in src.iter_mut().enumerate() {
            *v = (i * 7 + 3) as u8;
        }
        for n in 0..=300 {
            for so in 0..4 {
                for d in 0..4 {
                    dst.fill(0xEE);
                    dst[d..d + n].copy_from_slice(&src[so..so + n]);
                    let ok = (0..n).all(|i| black_box(dst[d + i]) == src[so + i])
                        && dst[..d].iter().chain(&dst[d + n..]).all(|&b| b == 0xEE);
                    if !ok {
                        return Err("memcpy");
                    }
                    if dst[d..d + n] != src[so..so + n] {
                        return Err("memcmp");
                    }
                }
            }
            for d in 0..4 {
                dst.fill(0x11);
                dst[d..d + n].fill(0xA5);
                if !(dst[d..d + n].iter().all(|&b| b == 0xA5)
                    && dst[..d].iter().chain(&dst[d + n..]).all(|&b| b == 0x11))
                {
                    return Err("memset");
                }
            }
        }
        for n in 0..=96 {
            for from in 0..8 {
                for to in 0..8 {
                    dst[..200].copy_from_slice(&src[..200]);
                    dst.copy_within(from..from + n, to);
                    if !(0..n).all(|i| dst[to + i] == src[from + i]) {
                        return Err("memmove");
                    }
                }
            }
        }
        Ok(())
    }

    fn naive_copy(dst: &mut [u8], src: &[u8]) {
        for (d, s) in dst.iter_mut().zip(src) {
            *d = black_box(*s);
        }
    }
}

#[cfg(target_os = "none")]
pub use target::*;
