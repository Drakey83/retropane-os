//! Memory primitives: the `memcpy` family every RetroPane image links.
//!
//! The compiler-builtins versions copy one word per four instructions on
//! ARMv5 (`ldr`/`str`/`cmp`/`b`). Here the aligned bulk case moves 32 bytes per
//! `ldm`/`stm` pair, with a `pld` prefetch for the XScale core; everything else
//! is plain Rust. See docs/perf.md for measurements.
//!
//! `#![no_builtins]` stops LLVM from turning these loops back into calls to
//! `memcpy` (which would recurse). On bare metal the functions are exported
//! under their C and ARM EABI names; on the host they're ordinary functions so
//! they can be tested against the standard library.

#![cfg_attr(not(test), no_std)]
#![no_builtins]

/// Copies `n` bytes forward. Regions may overlap only if `dst <= src`.
///
/// # Safety
/// `src` must be valid for `n` reads and `dst` for `n` writes.
pub unsafe fn copy_forward(mut dst: *mut u8, mut src: *const u8, mut n: usize) {
    // SAFETY: the caller guarantees both ranges are valid; every access below
    // stays within `dst..dst+n` / `src..src+n`.
    unsafe {
        if n >= 16 {
            // Align the destination.
            while (dst as usize) & 3 != 0 {
                *dst = *src;
                dst = dst.add(1);
                src = src.add(1);
                n -= 1;
            }
            if (src as usize) & 3 == 0 {
                let bulk = n & !31;
                copy_aligned_32(dst.cast(), src.cast(), bulk);
                dst = dst.add(bulk);
                src = src.add(bulk);
                n -= bulk;
                while n >= 4 {
                    *dst.cast::<u32>() = *src.cast::<u32>();
                    dst = dst.add(4);
                    src = src.add(4);
                    n -= 4;
                }
            } else {
                // Source misaligned: read aligned words and merge.
                let shift = ((src as usize) & 3) * 8;
                let mut s = src.sub((src as usize) & 3).cast::<u32>();
                let mut prev = *s;
                while n >= 4 {
                    s = s.add(1);
                    let next = *s;
                    *dst.cast::<u32>() = (prev >> shift) | (next << (32 - shift));
                    prev = next;
                    dst = dst.add(4);
                    src = src.add(4);
                    n -= 4;
                }
            }
        }
        while n > 0 {
            *dst = *src;
            dst = dst.add(1);
            src = src.add(1);
            n -= 1;
        }
    }
}

/// Copies `n` bytes backward (for overlapping moves with `dst > src`).
///
/// # Safety
/// As [`copy_forward`].
pub unsafe fn copy_backward(dst: *mut u8, src: *const u8, mut n: usize) {
    // SAFETY: the caller guarantees both ranges are valid; accesses stay inside.
    unsafe {
        let (mut d, mut s) = (dst.add(n), src.add(n));
        if n >= 16 && (d as usize) & 3 == (s as usize) & 3 {
            while (d as usize) & 3 != 0 {
                d = d.sub(1);
                s = s.sub(1);
                *d = *s;
                n -= 1;
            }
            while n >= 4 {
                d = d.sub(4);
                s = s.sub(4);
                *d.cast::<u32>() = *s.cast::<u32>();
                n -= 4;
            }
        }
        while n > 0 {
            d = d.sub(1);
            s = s.sub(1);
            *d = *s;
            n -= 1;
        }
    }
}

/// Fills `n` bytes with `c`.
///
/// # Safety
/// `dst` must be valid for `n` writes.
pub unsafe fn set(mut dst: *mut u8, c: u8, mut n: usize) {
    // SAFETY: the caller guarantees the range is valid; writes stay inside.
    unsafe {
        if n >= 16 {
            while (dst as usize) & 3 != 0 {
                *dst = c;
                dst = dst.add(1);
                n -= 1;
            }
            let word = u32::from(c) * 0x0101_0101;
            let bulk = n & !31;
            set_aligned_32(dst.cast(), word, bulk);
            dst = dst.add(bulk);
            n -= bulk;
            while n >= 4 {
                *dst.cast::<u32>() = word;
                dst = dst.add(4);
                n -= 4;
            }
        }
        while n > 0 {
            *dst = c;
            dst = dst.add(1);
            n -= 1;
        }
    }
}

/// Compares `n` bytes; returns <0, 0 or >0 like C `memcmp`.
///
/// # Safety
/// Both pointers must be valid for `n` reads.
pub unsafe fn compare(a: *const u8, b: *const u8, n: usize) -> i32 {
    // SAFETY: the caller guarantees both ranges are valid.
    unsafe {
        let mut i = 0;
        if (a as usize) & 3 == 0 && (b as usize) & 3 == 0 {
            while i + 4 <= n && *a.add(i).cast::<u32>() == *b.add(i).cast::<u32>() {
                i += 4;
            }
        }
        while i < n {
            let (x, y) = (*a.add(i), *b.add(i));
            if x != y {
                return i32::from(x) - i32::from(y);
            }
            i += 1;
        }
        0
    }
}

/// Copies `bytes` (a multiple of 32) between word-aligned buffers.
#[inline(always)]
unsafe fn copy_aligned_32(dst: *mut u32, src: *const u32, bytes: usize) {
    #[cfg(all(target_arch = "arm", target_os = "none"))]
    if bytes > 0 {
        // SAFETY: both buffers are word-aligned and valid for `bytes`, a
        // non-zero multiple of 32; `pld` is a hint and never faults.
        unsafe {
            core::arch::asm!(
                "2:",
                "pld [r1, #64]",
                "ldmia r1!, {{r3, r4, r5, r7, r8, r9, r10, r12}}",
                "subs r2, r2, #32",
                "stmia r0!, {{r3, r4, r5, r7, r8, r9, r10, r12}}",
                "bne 2b",
                // r6 and r11 are reserved by LLVM on this target.
                inout("r0") dst => _,
                inout("r1") src => _,
                inout("r2") bytes => _,
                out("r3") _, out("r4") _, out("r5") _, out("r7") _,
                out("r8") _, out("r9") _, out("r10") _, out("r12") _,
                options(nostack),
            );
        }
    }
    #[cfg(not(all(target_arch = "arm", target_os = "none")))]
    // SAFETY: as above; the host version copies word by word.
    unsafe {
        for i in 0..bytes / 4 {
            *dst.add(i) = *src.add(i);
        }
    }
}

/// Fills `bytes` (a multiple of 32) of a word-aligned buffer with `word`.
#[inline(always)]
unsafe fn set_aligned_32(dst: *mut u32, word: u32, bytes: usize) {
    #[cfg(all(target_arch = "arm", target_os = "none"))]
    if bytes > 0 {
        // SAFETY: the buffer is word-aligned and valid for `bytes`, a non-zero
        // multiple of 32.
        unsafe {
            core::arch::asm!(
                "mov r4, r3", "mov r5, r3", "mov r7, r3", "mov r8, r3",
                "mov r9, r3", "mov r10, r3", "mov r12, r3",
                "2:",
                "subs r2, r2, #32",
                "stmia r0!, {{r3, r4, r5, r7, r8, r9, r10, r12}}",
                "bne 2b",
                // r6 and r11 are reserved by LLVM on this target.
                inout("r0") dst => _,
                inout("r2") bytes => _,
                in("r3") word,
                out("r4") _, out("r5") _, out("r7") _, out("r8") _,
                out("r9") _, out("r10") _, out("r12") _,
                options(nostack),
            );
        }
    }
    #[cfg(not(all(target_arch = "arm", target_os = "none")))]
    // SAFETY: as above; the host version stores word by word.
    unsafe {
        for i in 0..bytes / 4 {
            *dst.add(i) = word;
        }
    }
}

/// The C / ARM EABI entry points, exported only in bare-metal builds.
#[cfg(all(target_arch = "arm", target_os = "none"))]
mod exports {
    use super::{compare, copy_backward, copy_forward, set};

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        // SAFETY: C contract: valid, non-overlapping ranges.
        unsafe { copy_forward(dst, src, n) };
        dst
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        // SAFETY: C contract: valid ranges, possibly overlapping; pick the
        // direction that never reads a byte after overwriting it.
        unsafe {
            if (dst as usize).wrapping_sub(src as usize) >= n {
                copy_forward(dst, src, n);
            } else {
                copy_backward(dst, src, n);
            }
        }
        dst
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memset(dst: *mut u8, c: i32, n: usize) -> *mut u8 {
        // SAFETY: C contract: valid range.
        unsafe { set(dst, c as u8, n) };
        dst
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
        // SAFETY: C contract: valid ranges.
        unsafe { compare(a, b, n) }
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
        // SAFETY: C contract: valid ranges.
        unsafe { compare(a, b, n) }
    }

    // ARM EABI helpers (RTABI §4.3.4). Note memset's argument order.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memcpy(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memcpy.
        unsafe { copy_forward(dst, src, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memcpy4(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memcpy.
        unsafe { copy_forward(dst, src, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memcpy8(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memcpy.
        unsafe { copy_forward(dst, src, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memmove(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memmove.
        unsafe { memmove(dst, src, n) };
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memmove4(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memmove.
        unsafe { memmove(dst, src, n) };
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memmove8(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memmove.
        unsafe { memmove(dst, src, n) };
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memset(dst: *mut u8, n: usize, c: i32) {
        // SAFETY: as memset.
        unsafe { set(dst, c as u8, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memset4(dst: *mut u8, n: usize, c: i32) {
        // SAFETY: as memset.
        unsafe { set(dst, c as u8, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memset8(dst: *mut u8, n: usize, c: i32) {
        // SAFETY: as memset.
        unsafe { set(dst, c as u8, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memclr(dst: *mut u8, n: usize) {
        // SAFETY: as memset.
        unsafe { set(dst, 0, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memclr4(dst: *mut u8, n: usize) {
        // SAFETY: as memset.
        unsafe { set(dst, 0, n) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memclr8(dst: *mut u8, n: usize) {
        // SAFETY: as memset.
        unsafe { set(dst, 0, n) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 300;

    fn source() -> Vec<u8> {
        (0..MAX + 8).map(|i| (i * 7 + 3) as u8).collect()
    }

    #[test]
    fn copy_forward_matches_std_for_all_sizes_and_alignments() {
        let src = source();
        for n in 0..=MAX {
            for so in 0..4 {
                for d in 0..4 {
                    let mut dst = vec![0xEEu8; MAX + 8];
                    // SAFETY: both ranges are in bounds and don't overlap.
                    unsafe { copy_forward(dst.as_mut_ptr().add(d), src.as_ptr().add(so), n) };
                    assert_eq!(&dst[d..d + n], &src[so..so + n], "n={n} so={so} d={d}");
                    assert!(dst[..d].iter().all(|&b| b == 0xEE), "wrote before dst");
                    assert!(dst[d + n..].iter().all(|&b| b == 0xEE), "wrote past dst");
                }
            }
        }
    }

    #[test]
    fn overlapping_moves_match_std_copy_within() {
        for n in 0..=96 {
            for from in 0..8 {
                for to in 0..8 {
                    let mut ours = source();
                    let mut want = source();
                    want.copy_within(from..from + n, to);
                    let base = ours.as_mut_ptr();
                    // SAFETY: both ranges are inside the vector.
                    unsafe {
                        let (d, s) = (base.add(to), base.add(from));
                        if (d as usize).wrapping_sub(s as usize) >= n {
                            copy_forward(d, s, n);
                        } else {
                            copy_backward(d, s, n);
                        }
                    }
                    assert_eq!(ours, want, "n={n} from={from} to={to}");
                }
            }
        }
    }

    #[test]
    fn set_matches_std_for_all_sizes_and_alignments() {
        for n in 0..=MAX {
            for d in 0..4 {
                let mut buf = vec![0x11u8; MAX + 8];
                // SAFETY: in bounds.
                unsafe { set(buf.as_mut_ptr().add(d), 0xA5, n) };
                assert!(buf[d..d + n].iter().all(|&b| b == 0xA5), "n={n} d={d}");
                assert!(buf[..d].iter().chain(&buf[d + n..]).all(|&b| b == 0x11));
            }
        }
    }

    #[test]
    fn compare_matches_std_ordering() {
        let a = source();
        for n in 0..64 {
            for flip in 0..n {
                let mut b = a.clone();
                b[flip] = b[flip].wrapping_add(1);
                // SAFETY: in bounds.
                let r = unsafe { compare(a.as_ptr(), b.as_ptr(), n) };
                assert_eq!(r.signum(), a[..n].cmp(&b[..n]) as i32, "n={n} flip={flip}");
            }
            // SAFETY: in bounds.
            assert_eq!(unsafe { compare(a.as_ptr(), a.as_ptr(), n) }, 0);
        }
    }
}
