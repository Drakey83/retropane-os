//! The XScale's media unit (iWMMXt) for hot loops (the performance gate's
//! C6; the owner's choice, 2026-10-22: a small HAL crate, not a hook).
//!
//! Each routine has one entry the caller uses everywhere: on the X51v it
//! takes the unit's path when the unit is reachable and the data suits it,
//! and the plain loop otherwise; on the host and anywhere else it is the
//! plain loop. Both give the same result (the probe checks the unit's
//! against the plain loop on the device, `rp-probe` `wmmx_dot`).
//!
//! **Never a fault.** A media instruction with the unit's access off is
//! undefined, so the unit's path runs only after whoever guarantees the
//! access says so with [`set_unit`]: the kernel once its lazy context
//! switching is set up (`rp_hal_pxa27x::iwmmxt`), a bare image after
//! `iwmmxt::single_access(true)`. Until then every call is the plain loop:
//! right, just slower.

#![no_std]

/// The unit is reachable from the code calling these routines (the
/// X51v's target has no atomics: a flag written at set-up and tear-down).
#[cfg(all(target_arch = "arm", target_os = "none"))]
static mut UNIT: bool = false;

/// Says whether the unit is reachable (see the module notes); the
/// routines take its path only while it is. Call at set-up or tear-down,
/// not while another context may be in a routine.
pub fn set_unit(on: bool) {
    #[cfg(all(target_arch = "arm", target_os = "none"))]
    // SAFETY: a plain write of a flag (single core; written only at set-up
    // and tear-down, read by the routines).
    unsafe {
        core::ptr::addr_of_mut!(UNIT).write_volatile(on);
    }
    #[cfg(not(all(target_arch = "arm", target_os = "none")))]
    let _ = on;
}

/// Whether the routines take the unit's path (never off the X51v).
#[must_use]
pub fn unit() -> bool {
    #[cfg(all(target_arch = "arm", target_os = "none"))]
    // SAFETY: a plain read of the flag [`set_unit`] writes.
    return unsafe { core::ptr::addr_of!(UNIT).read_volatile() };
    #[cfg(not(all(target_arch = "arm", target_os = "none")))]
    false
}

/// `sum(win[at + k] * coef[k])` over `coef`, in wrapping 32-bit sums;
/// by the unit when it is reachable and [`wmmx`]'s conditions hold
/// (`win` and `coef` 8-aligned, `coef` a nonzero multiple of 8 long, the
/// words read inside `win`), else [`dot_plain`]. `None` if `coef` reaches
/// past the end of `win`.
#[must_use]
pub fn dot(win: &[i16], at: usize, coef: &[i16]) -> Option<i32> {
    if win.len() < coef.len() || at > win.len() - coef.len() {
        return None;
    }
    #[cfg(all(target_arch = "arm", target_os = "none"))]
    if unit()
        && let Some(s) = wmmx(win, at, coef)
    {
        return Some(s);
    }
    Some(dot_plain(&win[at..at + coef.len()], coef))
}

/// The plain loop: `sum(x[k] * c[k])`, wrapping 32-bit sums.
#[must_use]
pub fn dot_plain(x: &[i16], c: &[i16]) -> i32 {
    x.iter().zip(c).fold(0i32, |s, (x, c)| {
        s.wrapping_add(i32::from(*x) * i32::from(*c))
    })
}

// The resampler's filter as one dot product (gate C6): 16-bit samples
// against 16-bit coefficients, four at a time. WMADDS wRd, wRn, wRm
// (`0xEEAn_d10m`) multiplies four pairs and adds them two by two into two
// 32-bit lanes; WADDW (`0xEE8n_d18m`) accumulates them; WALIGNR0
// (`0xEE8n_d02m`) takes 8 bytes from offset `wCGR0` of wRn:wRm for a window
// that isn't 8-aligned. An aligned window takes its own loop: QEMU's
// WALIGNR at offset 0 shifts by 64 (undefined in C; on x86 it ORs the two
// registers), where the device gives wRn.
#[cfg(all(target_arch = "arm", target_os = "none"))]
core::arch::global_asm!(
    r#"
    .section .text.rp_wmmx_dot, "ax"
    .arm
    @ rp_wmmx_dot(win, coef, pairs) -> i32: sum of win[k] * coef[k] for
    @ k < 8 * pairs; coef 8-aligned, pairs > 0. Two chains interleaved
    @ (wR3/wR4/wR15 and wR7/wR8/wR14) to hide the load-to-multiply-to-add
    @ latencies.
    .global rp_wmmx_dot
rp_wmmx_dot:
    mov     r12, #0
    mcrr    p0, #0, r12, r12, c15       @ wR15 = 0
    mcrr    p0, #0, r12, r12, c14       @ wR14 = 0
    ands    r3, r0, #7
    bic     r0, r0, #7
    beq     2f
    mcr     p1, #0, r3, c8, c0, #0      @ wCGR0 = the window's offset
    ldcl    p1, c0, [r0], #8
1:  ldcl    p1, c1, [r0], #8
    ldcl    p1, c5, [r0], #8
    ldcl    p1, c2, [r1], #8
    ldcl    p1, c6, [r1], #8
    .inst   0xEE803021                  @ WALIGNR0 wR3, wR0, wR1
    .inst   0xEE817025                  @ WALIGNR0 wR7, wR1, wR5
    .inst   0xEEA34102                  @ WMADDS wR4, wR3, wR2
    .inst   0xEEA78106                  @ WMADDS wR8, wR7, wR6
    .inst   0xEE050005                  @ WOR wR0, wR5, wR5
    .inst   0xEE8FF184                  @ WADDW wR15, wR15, wR4
    .inst   0xEE8EE188                  @ WADDW wR14, wR14, wR8
    subs    r2, r2, #1
    bne     1b
    b       3f
2:  ldcl    p1, c3, [r0], #8
    ldcl    p1, c7, [r0], #8
    ldcl    p1, c2, [r1], #8
    ldcl    p1, c6, [r1], #8
    .inst   0xEEA34102                  @ WMADDS wR4, wR3, wR2
    .inst   0xEEA78106                  @ WMADDS wR8, wR7, wR6
    .inst   0xEE8FF184                  @ WADDW wR15, wR15, wR4
    .inst   0xEE8EE188                  @ WADDW wR14, wR14, wR8
    subs    r2, r2, #1
    bne     2b
3:  .inst   0xEE8FF18E                  @ WADDW wR15, wR15, wR14
    mrrc    p0, #0, r0, r1, c15
    add     r0, r0, r1
    bx      lr
    "#
);

#[cfg(all(target_arch = "arm", target_os = "none"))]
unsafe extern "C" {
    fn rp_wmmx_dot(win: *const i16, coef: *const i16, pairs: usize) -> i32;
}

/// The unit's dot product: `sum(win[at + k] * coef[k])` over `coef`
/// (wrapping 32-bit sums); `None` (nothing read) unless `win` and `coef`
/// start 8-aligned, `coef` is a nonzero multiple of 8 long, and the whole
/// 8-byte words the routine reads lie inside `win`. The unit must be on
/// (the probe's direct use; everyone else calls [`dot`]).
#[cfg(all(target_arch = "arm", target_os = "none"))]
#[must_use]
pub fn wmmx(win: &[i16], at: usize, coef: &[i16]) -> Option<i32> {
    let (wb, cb) = (win.as_ptr() as usize, coef.as_ptr() as usize);
    if wb % 8 != 0 || cb % 8 != 0 || coef.is_empty() || coef.len() % 8 != 0 {
        return None;
    }
    // Bytes read: from `at`'s 8-byte word to the end of the word after the
    // last (one more word when `at` isn't aligned).
    let first = (2 * at) & !7;
    let words = coef.len() / 4 + usize::from((2 * at) % 8 != 0);
    if first + 8 * words > 2 * win.len() {
        return None;
    }
    // SAFETY: both start 8-aligned (checked); the routine reads `words`
    // 8-byte words of `win` from byte `first` (inside it, checked) and
    // `coef.len()` samples of `coef`.
    Some(unsafe { rp_wmmx_dot(win.as_ptr().add(at), coef.as_ptr(), coef.len() / 8) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plain path against a naive sum at every start and length,
    /// wrapping as the unit's does; out of range is `None`.
    #[test]
    fn dot_is_the_sum_of_products_everywhere() {
        let win: [i16; 64] = core::array::from_fn(|k| (k as i16).wrapping_mul(2_731) ^ 0x5A5A);
        let coef: [i16; 48] =
            core::array::from_fn(|k| (k as i16).wrapping_mul(1_237).wrapping_sub(20_000));
        for len in 0..=48 {
            for at in 0..=64 - len {
                let want = (0..len).fold(0i32, |s, k| {
                    s.wrapping_add(i32::from(win[at + k]) * i32::from(coef[k]))
                });
                assert_eq!(dot(&win, at, &coef[..len]), Some(want), "at {at} len {len}");
            }
        }
        assert_eq!(dot(&win, 17, &coef), None);
        assert_eq!(dot(&win[..8], 0, &coef[..16]), None);
    }

    /// Wrapping, not saturating or panicking: the unit's sums wrap.
    #[test]
    fn dot_wraps_as_the_unit_does() {
        let x = [i16::MIN; 8];
        let want = (0..8).fold(0i32, |s, _| {
            s.wrapping_add(i32::from(i16::MIN) * i32::from(i16::MIN))
        });
        assert_eq!(dot(&x, 0, &x), Some(want));
    }
}
