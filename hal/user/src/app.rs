//! The app-program runtime (the owner, 2026-11-14: every app its own
//! program; docs/design/app-scope.md).
//!
//! An app is a PE user image like any process (`hal/user/user.x`,
//! `xtask/src/userimg.rs`). Until the shell runs on the kernel, the shell
//! itself loads it (relocated by `rp-kernel-pe` into memory it owns) and
//! calls its entry once with an opening record; the entry puts the app's
//! state in its own `.bss` ([`AppSlot`]) and hands back the program. Under
//! the kernel the same image becomes a process and the same program is
//! driven over IPC (ADR-0016); only this runtime changes.
//!
//! - [`app_entry!`]: the entry the shell calls, `rp_app_entry(&mut T)`.
//! - With the `app` feature, `_start` (the PE's entry point) branches to
//!   it, and the panic handler stops on an undefined instruction, which the
//!   shell's fault handler reports with the app's address (in the test
//!   stage an app shares the shell's address space: quality audit C99).

use core::cell::{Cell, UnsafeCell};
use core::mem::MaybeUninit;

/// One value of `T` in a program's `.bss`, made once: the app's state.
/// The image is loaded afresh (its `.bss` zeroed) each time it opens, so
/// "once" is once per opening.
pub struct AppSlot<T> {
    taken: Cell<bool>,
    value: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: the value is reached only through the one `&'static mut` that
// `put_with` hands out, at most once (the flag), so it is never shared; an
// app program runs on the shell's one thread (ARMv5 has no atomics, and
// nothing here needs them).
unsafe impl<T> Sync for AppSlot<T> {}

impl<T> AppSlot<T> {
    /// An empty slot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            taken: Cell::new(false),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// The value made by `make`, the first time; `None` after.
    // The one `&'static mut` ever made of the value (the flag): the lint
    // sees only a shared `self` lending a unique borrow (as
    // rp_hal_x51v::cardsink's).
    #[allow(clippy::mut_from_ref)]
    pub fn put_with(&'static self, make: impl FnOnce() -> T) -> Option<&'static mut T> {
        if self.taken.replace(true) {
            return None;
        }
        // SAFETY: the flag was clear, so no reference to the value exists,
        // and none will be made again; the cell lives as long as the image.
        let cell = unsafe { &mut *self.value.get() };
        Some(cell.write(make()))
    }

    /// The value made by `init` straight into the slot (it writes it
    /// through the `MaybeUninit`, a field at a time, and returns it), for
    /// an app's state too big to be made on the stack and moved (the stack
    /// rule, P6: Contacts' 31 KB); `None` after the first time.
    #[allow(clippy::mut_from_ref)]
    pub fn put_in(
        &'static self,
        init: impl FnOnce(&'static mut MaybeUninit<T>) -> &'static mut T,
    ) -> Option<&'static mut T> {
        if self.taken.replace(true) {
            return None;
        }
        // SAFETY: as `put_with`: the only reference ever made.
        Some(init(unsafe { &mut *self.value.get() }))
    }
}

impl<T> Default for AppSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The entry the shell calls: `app_entry!(Opening, open)` makes
/// `rp_app_entry(&mut Opening)` call `open`.
#[macro_export]
macro_rules! app_entry {
    ($t:ty, $f:path) => {
        #[unsafe(no_mangle)]
        extern "Rust" fn rp_app_entry(o: &mut $t) {
            $f(o)
        }
    };
}

#[cfg(all(target_os = "none", feature = "app"))]
core::arch::global_asm!(
    r#"
    .section .text._start, "ax"
    .arm
    .global _start
    @ The shell calls the entry with r0 = the opening record and returns
    @ through lr: the branch keeps both.
_start:
    b       rp_app_entry
"#
);

/// An app's fault: stops on `UDF`, which the shell's fault handler reports
/// with the address (the test stage has no process to end). Never unwinds.
#[cfg(all(target_os = "none", feature = "app"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: a permanently undefined instruction; the fault handler never
    // returns here.
    unsafe { core::arch::asm!("udf #0", options(noreturn)) }
}
