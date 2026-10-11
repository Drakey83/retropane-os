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

    /// Polled SD reads: one block per command, and four per command
    /// (CMD18), where the card's per-command start-up cost is paid once.
    pub fn sd<H: SdHost>(r: &mut Runner, card: &mut SdCard<H>) {
        let mut block = [0u8; BLOCK];
        let mut lba = 0;
        r.run("sd_read_block", 64, &mut || {
            let _ = card.read(lba, &mut block);
            lba = (lba + 1) % 64;
        });
        let mut four = [0u8; 4 * BLOCK];
        let mut lba = 0;
        r.run("sd_read_4_blocks", 16, &mut || {
            let _ = card.read_many(lba, &mut four);
            lba = (lba + 4) % 64;
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
