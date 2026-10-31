//! A store of pictures made at run time (option C of the Weather rule 0
//! redo, docs/design/weather-rule0.md): fixed slots carved once from one
//! block of RAM, and small handles to them.
//!
//! Borrowed: Windows' handles carry uniqueness bits, so a handle to a
//! freed object is refused rather than reaching the next one; Android's
//! renderer keys its GPU copy of a bitmap by the bitmap's generation, so a
//! changed bitmap is uploaded again. Ours: the slots' sizes are the
//! shell's picture sizes, known at build time, so the store never
//! fragments and needs no allocator; a slot's generation moves on at every
//! reuse, so a stale handle draws nothing (never another picture) and a
//! renderer's copy keyed by [`Pic::key`] is never shown stale; no
//! `unsafe` (the slots are `RefCell`s: the image writes a picture, a
//! renderer reads it, never both at once). The shell holds handles only
//! (ADR-0021 rule 1); the pixels stay in SDRAM, where the CPU composes
//! the pages (ADR-0022 rule 2); a renderer reaches them by handle, never
//! by address (ADR-0030 G3).

use core::cell::{Cell, RefCell};

use crate::Argb;

/// Slots a store holds at most: every condition at every size the shell
/// shows (22 x 4, the previews' whole set) and room over (the device's
/// store has 28; each slot costs 20 bytes here).
pub const SLOTS: usize = 96;

/// A picture in a [`Pics`] store: its slot, the slot's generation when it
/// was made, and its size. Plain data: copied freely, checked on use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pic {
    slot: u8,
    era: u16,
    /// Width.
    pub w: u16,
    /// Height.
    pub h: u16,
}

impl Pic {
    /// A key unique to this picture while the store lives (slot and
    /// generation): what a renderer's copies are kept by.
    #[must_use]
    pub const fn key(&self) -> u32 {
        (self.slot as u32) << 16 | self.era as u32
    }
}

/// One slot: its pixels, the largest picture it takes, its generation and
/// whether a picture is in it.
struct Slot<'a> {
    px: RefCell<&'a mut [Argb]>,
    w: u16,
    h: u16,
    era: Cell<u16>,
    used: Cell<bool>,
    /// Read since it was made: its pixels are final (a renderer may hold a
    /// copy keyed by its handle).
    sealed: Cell<bool>,
}

/// The store: slots of the sizes asked for, from one block.
pub struct Pics<'a> {
    slots: [Option<Slot<'a>>; SLOTS],
}

impl<'a> Pics<'a> {
    /// Carves `block` into `classes` (the largest picture a slot of the
    /// class takes, width and height, and how many such slots); slots that don't
    /// fit in `block`, or past [`SLOTS`], are left out ([`Self::slots`]
    /// says how many were made).
    #[must_use]
    pub fn new(block: &'a mut [Argb], classes: &[(u16, u16, u8)]) -> Self {
        let mut p = Self {
            slots: [const { None }; SLOTS],
        };
        p.carve(block, classes);
        p
    }

    /// [`Pics::new`] made straight into `slot` (the image's, in `.bss`): at
    /// [`SLOTS`] slots the store is 1.9 KB, and made on the stack and moved
    /// it took a 4,640-byte frame (`cargo xtask stacks`, P6).
    pub fn new_in<'s>(
        slot: &'s mut core::mem::MaybeUninit<Self>,
        block: &'a mut [Argb],
        classes: &[(u16, u16, u8)],
    ) -> &'s mut Self {
        let p = rp_hal_mem::in_place!(slot, Pics { slots: @each None });
        p.carve(block, classes);
        p
    }

    fn carve(&mut self, block: &'a mut [Argb], classes: &[(u16, u16, u8)]) {
        let mut rest = block;
        let mut n = 0;
        for &(w, h, count) in classes {
            let words = usize::from(w) * usize::from(h);
            for _ in 0..count {
                if n == SLOTS || rest.len() < words {
                    break;
                }
                let (mine, more) = core::mem::take(&mut rest).split_at_mut(words);
                rest = more;
                self.slots[n] = Some(Slot {
                    px: RefCell::new(mine),
                    w,
                    h,
                    era: Cell::new(0),
                    used: Cell::new(false),
                    sealed: Cell::new(false),
                });
                n += 1;
            }
        }
    }

    /// Words a store of `classes` needs.
    #[must_use]
    pub const fn words(classes: &[(u16, u16, u8)]) -> usize {
        let mut i = 0;
        let mut n = 0;
        while i < classes.len() {
            let (w, h, count) = classes[i];
            n += w as usize * h as usize * count as usize;
            i += 1;
        }
        n
    }

    /// Slots made.
    #[must_use]
    pub fn slots(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// Slots in use.
    #[must_use]
    pub fn used(&self) -> usize {
        self.slots.iter().flatten().filter(|s| s.used.get()).count()
    }

    /// Room for a `w` x `h` picture: the free slot with the least room
    /// that takes it; `None` when every such slot is in use. Its pixels are
    /// cleared (see-through) until written.
    pub fn alloc(&self, w: u16, h: u16) -> Option<Pic> {
        let (k, s) = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(k, s)| Some((k, s.as_ref()?)))
            .filter(|(_, s)| !s.used.get() && s.w >= w && s.h >= h)
            .min_by_key(|(_, s)| u32::from(s.w) * u32::from(s.h))?;
        s.used.set(true);
        s.sealed.set(false);
        let era = s.era.get().wrapping_add(1);
        s.era.set(era);
        s.px.try_borrow_mut().ok()?.fill(0);
        Some(Pic {
            slot: k as u8,
            era,
            w,
            h,
        })
    }

    /// The slot behind `pic` while `pic` is its current picture.
    fn live(&self, pic: Pic) -> Option<&Slot<'a>> {
        let s = self.slots.get(usize::from(pic.slot))?.as_ref()?;
        (s.used.get() && s.era.get() == pic.era).then_some(s)
    }

    /// Writes `pic`'s pixels (`w * h`, rows `w` apart) with `f`; false for
    /// a stale handle, while the pixels are being read, or once they have
    /// been read (a renderer may keep a copy by the handle: a changed
    /// picture is a new one, `free` then `alloc`).
    pub fn write(&self, pic: Pic, f: impl FnOnce(&mut [Argb])) -> bool {
        let Some(s) = self.live(pic) else {
            return false;
        };
        if s.sealed.get() {
            return false;
        }
        let Ok(mut px) = s.px.try_borrow_mut() else {
            return false;
        };
        let n = usize::from(pic.w) * usize::from(pic.h);
        match px.get_mut(..n) {
            Some(p) => {
                f(p);
                true
            }
            None => false,
        }
    }

    /// Reads `pic`'s pixels (`w * h`, rows `w` apart) with `f`; `None` for
    /// a stale handle or while they are being written.
    pub fn read<R>(&self, pic: Pic, f: impl FnOnce(&[Argb]) -> R) -> Option<R> {
        let s = self.live(pic)?;
        let px = s.px.try_borrow().ok()?;
        s.sealed.set(true);
        let n = usize::from(pic.w) * usize::from(pic.h);
        px.get(..n).map(f)
    }

    /// Gives `pic`'s slot back: the handle (and every copy of it) is stale
    /// from now on. A stale handle does nothing.
    pub fn free(&self, pic: Pic) {
        if let Some(s) = self.live(pic) {
            s.used.set(false);
            s.era.set(s.era.get().wrapping_add(1));
        }
    }

    /// Gives every slot back: every handle so far is stale.
    pub fn clear(&self) {
        for s in self.slots.iter().flatten() {
            if s.used.get() {
                s.used.set(false);
                s.era.set(s.era.get().wrapping_add(1));
            }
        }
    }

    /// Whether `pic` is still a picture in the store.
    #[must_use]
    pub fn is_live(&self, pic: Pic) -> bool {
        self.live(pic).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASSES: [(u16, u16, u8); 2] = [(4, 4, 2), (8, 8, 1)];

    fn store() -> Pics<'static> {
        Pics::new(
            Box::leak(vec![0; Pics::words(&CLASSES)].into_boxed_slice()),
            &CLASSES,
        )
    }

    #[test]
    fn a_picture_goes_in_the_smallest_slot_that_takes_it() {
        let s = store();
        assert_eq!(s.slots(), 3);
        let big = s.alloc(6, 8).expect("the 8 px slot");
        assert!(s.alloc(5, 5).is_none(), "the only slot over 4 px is taken");
        let a = s.alloc(4, 3).expect("a 4 px slot");
        let b = s.alloc(2, 2).expect("the other");
        assert!(s.alloc(1, 1).is_none());
        assert_eq!(s.used(), 3);
        assert_ne!(a.key(), b.key());
        assert_ne!(a.key(), big.key());
    }

    #[test]
    fn written_pixels_read_back_and_a_freed_handle_is_dead() {
        let s = store();
        let p = s.alloc(3, 2).unwrap();
        assert!(s.write(p, |px| px.copy_from_slice(&[1, 2, 3, 4, 5, 6])));
        assert_eq!(s.read(p, |px| px[4]), Some(5));
        assert!(
            !s.write(p, |px| px[4] = 9),
            "read once, its pixels are final"
        );
        assert_eq!(s.read(p, |px| px[4]), Some(5));
        s.free(p);
        assert!(!s.is_live(p));
        assert_eq!(s.read(p, |px| px[0]), None, "a stale handle reads nothing");
        assert!(!s.write(p, |px| px[0] = 9), "and writes nothing");
        // The slot again: a new generation, so a new key, and the old
        // handle stays dead though the slot is in use.
        let q = s.alloc(3, 2).unwrap();
        assert_ne!(q.key(), p.key());
        assert_eq!(q.key() >> 16, p.key() >> 16, "the same slot");
        assert!(!s.is_live(p));
        assert_eq!(s.read(q, |px| px[4]), Some(0), "cleared for the new one");
        let r = s.alloc(1, 1).unwrap();
        assert!(s.write(r, |px| px[0] = 3), "a new picture is open again");
        s.free(p);
        assert!(s.is_live(q), "freeing a stale handle leaves the new one");
    }

    #[test]
    fn a_picture_being_read_is_not_written() {
        let s = store();
        let p = s.alloc(2, 2).unwrap();
        let wrote = s.read(p, |_| s.write(p, |px| px[0] = 7));
        assert_eq!(wrote, Some(false));
        assert_eq!(s.read(p, |px| px[0]), Some(0));
    }

    #[test]
    fn slots_past_the_block_are_left_out() {
        let s = Pics::new(Box::leak(vec![0; 40].into_boxed_slice()), &CLASSES);
        assert_eq!(s.slots(), 2, "two 16-word slots fit in 40, the 64 doesn't");
        assert!(s.alloc(8, 8).is_none());
    }
}
