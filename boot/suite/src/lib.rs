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
        }

        /// Reports a self-check.
        pub fn check(&mut self, name: &'static str, ok: bool, detail: &str) {
            self.sink.check(name, ok, detail);
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

    /// LZ4 on 4 KiB pages (ADR-0005: compressed RAM and storage), three
    /// kinds: a heap-shaped stress page (pointers and small integers: ~1000
    /// minimum-length matches), text (UI strings and resources: long
    /// matches), and noise (already-compressed data: nothing to find). The
    /// round trip is checked on the CPU for each. `scratch` needs 16 KiB.
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
            let n = rp_lz4::compress(page, packed).unwrap_or(0);
            let ok =
                n > 0 && rp_lz4::decompress(&packed[..n], back) == Ok(4096) && back == &page[..];
            r.check(check, ok, if ok { "" } else { "round trip" });
            r.run(comp, 16, &mut || {
                black_box(rp_lz4::compress(black_box(&page[..]), packed).ok());
            });
            r.run(decomp, 64, &mut || {
                black_box(rp_lz4::decompress(black_box(&packed[..n]), back).ok());
            });
        }
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
