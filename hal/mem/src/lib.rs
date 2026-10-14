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

/// Views words as bytes (little-endian order on the X51v). Always sound:
/// `u8` has no alignment requirement and every bit pattern is a valid `u8`,
/// and the borrow keeps the words untouchable while the bytes are in use.
/// Lets the kernel treat a frame of its word-typed RAM pool as a page of
/// bytes without `unsafe` outside the HAL.
pub fn words_as_bytes_mut(words: &mut [u32]) -> &mut [u8] {
    let len = words.len() * 4;
    // SAFETY: the pointer comes from a valid `&mut [u32]` covering exactly
    // `len` bytes; u8 has alignment 1 and no invalid values; the returned
    // slice reborrows `words` mutably for its whole lifetime, so nothing
    // else can observe the words meanwhile.
    unsafe { core::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), len) }
}

/// Views 16-bit pixels as bytes (little-endian on the X51v), for copying a
/// composed RAM surface into the framebuffer with the burst `memcpy`.
/// Always sound, for the same reasons as [`words_as_bytes_mut`].
pub fn pixels_as_bytes(px: &[u16]) -> &[u8] {
    // SAFETY: the pointer comes from a valid `&[u16]` covering exactly
    // `2 * len` bytes; u8 has alignment 1 and no invalid values; the
    // returned slice borrows `px` for its whole lifetime.
    unsafe { core::slice::from_raw_parts(px.as_ptr().cast::<u8>(), px.len() * 2) }
}

/// Views 16-bit pixels as mutable bytes, for filling with [`fill32`]'s burst
/// stores. Always sound, as [`words_as_bytes_mut`].
pub fn pixels_as_bytes_mut(px: &mut [u16]) -> &mut [u8] {
    let len = px.len() * 2;
    // SAFETY: as `words_as_bytes_mut`: a valid `&mut [u16]` covering exactly
    // `len` bytes, reborrowed mutably for the result's whole lifetime.
    unsafe { core::slice::from_raw_parts_mut(px.as_mut_ptr().cast::<u8>(), len) }
}

/// Views bytes as 16-bit pixels, if they start 2-byte aligned (`None`
/// otherwise); an odd last byte is left out. For using a byte region (scratch
/// RAM) as a drawing surface.
pub fn bytes_as_pixels_mut(bytes: &mut [u8]) -> Option<&mut [u16]> {
    if bytes.as_ptr() as usize & 1 != 0 {
        return None;
    }
    let len = bytes.len() / 2;
    // SAFETY: the start is 2-byte aligned (checked), the range covers
    // `2 * len <= bytes.len()` bytes of a valid `&mut [u8]`, every bit
    // pattern is a valid u16, and the result reborrows `bytes` mutably for
    // its whole lifetime.
    Some(unsafe { core::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<u16>(), len) })
}

/// Views words as bytes, read-only: copying a composed 32-bit layer into the
/// framebuffer with the burst `memcpy`. Always sound, as
/// [`words_as_bytes_mut`].
pub fn words_as_bytes(words: &[u32]) -> &[u8] {
    // SAFETY: the pointer comes from a valid `&[u32]` covering exactly
    // `4 * len` bytes; u8 has alignment 1 and no invalid values; the
    // returned slice borrows `words` for its whole lifetime.
    unsafe { core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * 4) }
}

/// Views bytes as words, if they start 4-byte aligned (`None` otherwise);
/// up to 3 trailing bytes are left out. For using a byte region (scratch
/// RAM) as a 32-bit drawing layer.
pub fn bytes_as_words_mut(bytes: &mut [u8]) -> Option<&mut [u32]> {
    if bytes.as_ptr() as usize & 3 != 0 {
        return None;
    }
    let len = bytes.len() / 4;
    // SAFETY: the start is 4-byte aligned (checked), the range covers
    // `4 * len <= bytes.len()` bytes of a valid `&mut [u8]`, every bit
    // pattern is a valid u32, and the result reborrows `bytes` mutably for
    // its whole lifetime.
    Some(unsafe { core::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<u32>(), len) })
}

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
            let words = n & !3;
            set_words(dst.cast(), u32::from(c) * 0x0101_0101, words);
            dst = dst.add(words);
            n -= words;
        }
        while n > 0 {
            *dst = c;
            dst = dst.add(1);
            n -= 1;
        }
    }
}

/// Fills `buf` with the little-endian bytes of `word` repeating from `buf[0]`
/// (so a 16-bit pixel `c` is filled with `word = c | c << 16`). Any alignment;
/// the aligned middle uses 32-byte burst stores.
pub fn fill32(buf: &mut [u8], word: u32) {
    let bytes = word.to_le_bytes();
    let head = ((4 - (buf.as_ptr() as usize & 3)) & 3).min(buf.len());
    for (i, b) in buf[..head].iter_mut().enumerate() {
        *b = bytes[i % 4];
    }
    // At the first aligned address the pattern is rotated by `head` bytes.
    let rotated = word.rotate_right(8 * head as u32);
    let body = &mut buf[head..];
    let words = body.len() & !3;
    // SAFETY: `body[..words]` is in bounds and starts word-aligned.
    unsafe { set_words(body.as_mut_ptr().cast(), rotated, words) };
    for (i, b) in body[words..].iter_mut().enumerate() {
        *b = bytes[(head + words + i) % 4];
    }
}

/// Reads the little-endian word at `buf[offset..offset + 4]` with one
/// volatile load, so polling memory another bus master writes (a GPU fence)
/// re-reads it every time. `None` if out of bounds or not word-aligned.
#[must_use]
pub fn load32_volatile(buf: &[u8], offset: usize) -> Option<u32> {
    let word = buf.get(offset..offset.checked_add(4)?)?;
    if !(word.as_ptr() as usize).is_multiple_of(4) {
        return None;
    }
    // SAFETY: four in-bounds bytes at a word-aligned address, borrowed.
    Some(unsafe { word.as_ptr().cast::<u32>().read_volatile() })
}

/// Writes `value` to the word at `buf[offset..offset + 4]` with one volatile
/// store. `None` if out of bounds or not word-aligned.
pub fn store32_volatile(buf: &mut [u8], offset: usize, value: u32) -> Option<()> {
    let word = buf.get_mut(offset..offset.checked_add(4)?)?;
    if !(word.as_ptr() as usize).is_multiple_of(4) {
        return None;
    }
    // SAFETY: four in-bounds bytes at a word-aligned address, borrowed mutably.
    unsafe { word.as_mut_ptr().cast::<u32>().write_volatile(value) };
    Some(())
}

/// Stores `word` over `bytes` (a multiple of 4) at word-aligned `dst`.
#[inline(always)]
unsafe fn set_words(dst: *mut u32, word: u32, bytes: usize) {
    let bulk = bytes & !31;
    // SAFETY: the caller guarantees `dst` is aligned and valid for `bytes`.
    unsafe {
        set_aligned_32(dst, word, bulk);
        for i in bulk / 4..bytes / 4 {
            *dst.add(i) = word;
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
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
unsafe fn copy_aligned_32(dst: *mut u32, src: *const u32, bytes: usize) {
    #[cfg(all(target_arch = "arm", target_os = "none"))]
    if bytes > 0 {
        // SAFETY: both buffers are word-aligned and valid for `bytes`, a
        // non-zero multiple of 32; `pld` is a hint and never faults.
        unsafe {
            core::arch::asm!(
                "2:",
                "pld [r1, #64]",
                "ldmia r1!, {{r3, r4, r5, r8, r9, r10, r12, lr}}",
                "subs r2, r2, #32",
                "stmia r0!, {{r3, r4, r5, r8, r9, r10, r12, lr}}",
                "bne 2b",
                // r6 and the frame pointer (r11 in ARM code, r7 in Thumb)
                // are reserved by LLVM; lr is free to clobber in both.
                inout("r0") dst => _,
                inout("r1") src => _,
                inout("r2") bytes => _,
                out("r3") _, out("r4") _, out("r5") _, out("r8") _,
                out("r9") _, out("r10") _, out("r12") _, out("lr") _,
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
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
unsafe fn set_aligned_32(dst: *mut u32, word: u32, bytes: usize) {
    #[cfg(all(target_arch = "arm", target_os = "none"))]
    if bytes > 0 {
        // SAFETY: the buffer is word-aligned and valid for `bytes`, a non-zero
        // multiple of 32.
        unsafe {
            core::arch::asm!(
                "mov r4, r3", "mov r5, r3", "mov r8, r3", "mov lr, r3",
                "mov r9, r3", "mov r10, r3", "mov r12, r3",
                "2:",
                "subs r2, r2, #32",
                "stmia r0!, {{r3, r4, r5, r8, r9, r10, r12, lr}}",
                "bne 2b",
                // r6 and the frame pointer are reserved; lr is free.
                inout("r0") dst => _,
                inout("r2") bytes => _,
                in("r3") word,
                out("r4") _, out("r5") _, out("r8") _, out("r9") _,
                out("r10") _, out("r12") _, out("lr") _,
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
    use core::ffi::{c_int, c_void};

    use super::{compare, copy_backward, copy_forward, set};

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
        // SAFETY: C contract: valid, non-overlapping ranges.
        unsafe { copy_forward(dst.cast(), src.cast(), n) };
        dst
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
        // SAFETY: C contract: valid ranges, possibly overlapping; pick the
        // direction that never reads a byte after overwriting it.
        unsafe {
            if (dst as usize).wrapping_sub(src as usize) >= n {
                copy_forward(dst.cast(), src.cast(), n);
            } else {
                copy_backward(dst.cast(), src.cast(), n);
            }
        }
        dst
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memset(dst: *mut c_void, c: c_int, n: usize) -> *mut c_void {
        // SAFETY: C contract: valid range.
        unsafe { set(dst.cast(), c as u8, n) };
        dst
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
        // SAFETY: C contract: valid ranges.
        unsafe { compare(a.cast(), b.cast(), n) }
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
        // SAFETY: C contract: valid ranges.
        unsafe { compare(a.cast(), b.cast(), n) }
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
        unsafe { memmove(dst.cast(), src.cast(), n) };
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memmove4(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memmove.
        unsafe { memmove(dst.cast(), src.cast(), n) };
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn __aeabi_memmove8(dst: *mut u8, src: *const u8, n: usize) {
        // SAFETY: as memmove.
        unsafe { memmove(dst.cast(), src.cast(), n) };
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

    #[test]
    fn volatile_word_access_checks_bounds_and_alignment() {
        let mut words = [0u32; 2];
        let buf = words_as_bytes_mut(&mut words);
        assert_eq!(store32_volatile(buf, 4, 0x1234_5678), Some(()));
        assert_eq!(load32_volatile(buf, 4), Some(0x1234_5678));
        assert_eq!(buf[4], 0x78, "little-endian");
        assert_eq!(load32_volatile(buf, 2), None, "unaligned");
        assert_eq!(load32_volatile(buf, 8), None, "past the end");
        assert_eq!(store32_volatile(buf, 6, 0), None);
        assert_eq!(load32_volatile(buf, usize::MAX - 1), None, "overflow");
    }

    #[test]
    fn pixels_view_as_bytes_and_back() {
        let px = [0x0201u16, 0x0403];
        assert_eq!(pixels_as_bytes(&px), &[1, 2, 3, 4]);
        let mut w = [0u32; 2];
        let bytes = words_as_bytes_mut(&mut w);
        let p = bytes_as_pixels_mut(bytes).unwrap();
        assert_eq!(p.len(), 4);
        p[1] = 0xBEEF;
        assert_eq!(w[0], 0xBEEF_0000);
        // Misaligned: refused, not a misaligned reference.
        let mut w = [0u32; 2];
        let bytes = words_as_bytes_mut(&mut w);
        assert!(bytes_as_pixels_mut(&mut bytes[1..]).is_none());
        assert_eq!(bytes_as_pixels_mut(&mut bytes[2..7]).unwrap().len(), 2);
    }

    #[test]
    fn words_view_as_little_endian_bytes() {
        let mut w = [0x0403_0201u32, 0x0807_0605];
        let b = words_as_bytes_mut(&mut w);
        assert_eq!(b, &[1, 2, 3, 4, 5, 6, 7, 8]);
        b[0] = 0xFF;
        assert_eq!(w[0], 0x0403_02FF);
        assert_eq!(words_as_bytes(&w)[..2], [0xFF, 2]);
        // Bytes as words: aligned only, trailing bytes left out.
        let mut w = [0u32; 3];
        let bytes = words_as_bytes_mut(&mut w);
        assert!(bytes_as_words_mut(&mut bytes[2..]).is_none());
        let words = bytes_as_words_mut(&mut bytes[4..11]).unwrap();
        assert_eq!(words.len(), 1);
        words[0] = 0xCAFE_F00D;
        assert_eq!(w[1], 0xCAFE_F00D);
    }

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
    fn fill32_repeats_the_pattern_from_the_first_byte() {
        let word = 0x4433_2211u32;
        for n in 0..=100 {
            for d in 0..4 {
                let mut buf = [0u8; 112];
                fill32(&mut buf[d..d + n], word);
                for i in 0..n {
                    assert_eq!(buf[d + i], word.to_le_bytes()[i % 4], "n={n} d={d} i={i}");
                }
                assert!(buf[..d].iter().chain(&buf[d + n..]).all(|&b| b == 0));
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
