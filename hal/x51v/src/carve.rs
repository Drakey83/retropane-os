//! Typed tables in static memory, for code that forbids `unsafe`: records
//! carved off memory a `take_*` handout gave (MTP's object handles,
//! ADR-0037 rule 7: in static memory, never on a stack).

/// Carves `count` values of `T`, each set to `init`, off the front of
/// `mem` (memory this module handed out), aligned for `T`; returns them and
/// the words after them, or `None` when they don't fit. For tables of
/// records in static memory (MTP's object handles, ADR-0037 rule 7) used by
/// code that forbids `unsafe`.
pub fn carve<T: Copy + 'static>(
    mem: &'static mut [u32],
    count: usize,
    init: T,
) -> Option<(&'static mut [T], &'static mut [u32])> {
    if count == 0 {
        return Some((&mut [], mem));
    }
    let skip = mem.as_ptr().align_offset(core::mem::align_of::<T>());
    let bytes = count.checked_mul(core::mem::size_of::<T>())?;
    let words = bytes.div_ceil(4);
    if skip > mem.len() || words > mem.len() - skip {
        return None;
    }
    let (_, rest) = mem.split_at_mut(skip);
    let (table, rest) = rest.split_at_mut(words);
    let p = table.as_mut_ptr().cast::<T>();
    for i in 0..count {
        // SAFETY: element `i` lies inside `table` (`words` covers `count`
        // values) and `p` is aligned for `T`; writing a `Copy` value over
        // plain words drops nothing.
        unsafe { p.add(i).write(init) };
    }
    // SAFETY: `count` initialised, aligned values inside `table`, which is
    // consumed here, so this is the one reference to that memory.
    Some((unsafe { core::slice::from_raw_parts_mut(p, count) }, rest))
}

/// Carves one `T` off the front of `mem`, aligned for it, and puts
/// `make()` there; returns it and the words after it, or `None` when it
/// doesn't fit. For a structure too big for a stack frame (MTP's responder
/// and its tables, ADR-0037 rule 7); it is never dropped.
pub fn carve_with<T: 'static>(
    mem: &'static mut [u32],
    make: impl FnOnce() -> T,
) -> Option<(&'static mut T, &'static mut [u32])> {
    let skip = mem.as_ptr().align_offset(core::mem::align_of::<T>());
    let words = core::mem::size_of::<T>().div_ceil(4);
    if skip > mem.len() || words > mem.len() - skip {
        return None;
    }
    let (_, rest) = mem.split_at_mut(skip);
    let (cell, rest) = rest.split_at_mut(words);
    let p = cell.as_mut_ptr().cast::<T>();
    // SAFETY: `cell` holds `size_of::<T>()` bytes at `p`, aligned for `T`;
    // writing over plain words drops nothing; `cell` is consumed here, so
    // the reference made from `p` is the only one to that memory.
    unsafe {
        p.write(make());
        Some((&mut *p, rest))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::carve;
    use std::boxed::Box;
    use std::vec;

    fn mem(words: usize) -> &'static mut [u32] {
        Box::leak(vec![0xDEAD_BEEFu32; words].into_boxed_slice())
    }

    #[test]
    fn carves_initialised_aligned_records_and_hands_back_the_rest() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        struct Rec {
            a: u64,
            b: u8,
        }
        let m = mem(100);
        let (t, rest) = carve(m, 5, Rec { a: 7, b: 1 }).unwrap();
        assert_eq!(t.len(), 5);
        assert!(t.iter().all(|r| *r == Rec { a: 7, b: 1 }));
        assert_eq!(t.as_ptr() as usize % core::mem::align_of::<Rec>(), 0);
        // 5 records of 16 bytes: 20 words, plus at most one skipped for
        // alignment.
        assert!(rest.len() == 80 || rest.len() == 79, "{}", rest.len());
        t[4].a = 9;
        rest[0] = 1;
        assert_eq!(t[4].a, 9);
        // Bytes: rounded up to whole words.
        let (bytes, rest) = carve(rest, 5, 0u8).unwrap();
        assert_eq!(bytes, [0u8; 5]);
        assert!(rest.len() == 78 || rest.len() == 77);
    }

    #[test]
    fn carves_one_value_made_in_place() {
        struct Big {
            a: [u64; 40],
            b: u8,
        }
        let (v, rest) = super::carve_with(mem(100), || Big { a: [3; 40], b: 9 }).unwrap();
        assert_eq!((v.a[39], v.b), (3, 9));
        assert_eq!(v as *const Big as usize % 8, 0);
        assert!(rest.len() == 100 - 82 || rest.len() == 100 - 83);
        assert!(super::carve_with(mem(81), || Big { a: [0; 40], b: 0 }).is_none());
    }

    #[test]
    fn refuses_what_does_not_fit_or_overflows() {
        // Two u64s fit in five words wherever the memory starts.
        assert!(carve(mem(5), 2, 0u64).is_some());
        assert!(carve(mem(4), 3, 0u64).is_none());
        assert!(carve(mem(4), usize::MAX, 0u64).is_none());
        assert!(carve(mem(0), 0, 0u64).is_some());
    }
}
