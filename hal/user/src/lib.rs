//! The user-mode runtime (ADR-0016): what a compiled RetroPane process
//! links to reach the kernel.
//!
//! - The system-call ABI: `SWI #0`, the number in `r7`, arguments in
//!   `r0`-`r3`, the result in `r0` (an error is `0xFFFFFFxx`).
//! - With the `runtime` feature, the process entry `_start`: the kernel
//!   starts a thread there with `r0 = 0` and `r1` = the thread's role, and
//!   enters a server there for each call with `r0` = the caller's slot
//!   (2-32) and `r1`-`r3` its arguments. The program supplies
//!   [`entry!`]'s function.
//! - [`Window`]: a device's registers the kernel mapped into this process
//!   (uncached, at its grant's fixed address), as a [`RegisterBus`] for
//!   the ordinary drivers.
//!
//! The kernel crates use the constants only; the entry and the window's
//! volatile accesses are this crate's only `unsafe` for user code.

#![no_std]

use rp_hal_api::RegisterBus;

pub mod app;

/// Call a port (`r0` handle, `r1`-`r3` arguments): the server's answer.
pub const SYS_CALL: u32 = 1;
/// End the call this activation serves, answering `r0`.
pub const SYS_RETURN: u32 = 2;
/// Hand a handle on (`r0` handle, `r1` slot, `r2` rights).
pub const SYS_GRANT: u32 = 4;
/// End the calling process (`r0` exit code).
pub const SYS_EXIT: u32 = 5;
/// Wait for a granted interrupt (`r0`: the line's number in the board's
/// grant): re-arms the line, blocks until it fires, and returns the pin's
/// level (0 or 1). Only the process the line is granted to may wait.
pub const SYS_IRQ_WAIT: u32 = 7;
/// A test program's report (`r0`-`r3`); its thread then ends.
pub const SYS_REPORT: u32 = 8;
/// Game Mode's game only (G7): a cache operation on its own memory (`r0`
/// the operation: 0 clean, 1 discard, 2 clean then discard, 3 drain the
/// write buffer; `r1` the address, `r2` the length, at most 64 KiB).
pub const SYS_CACHE: u32 = 9;
/// Game Mode's game only (G7): start DMA channel `r0` (one the game holds)
/// on the chain at `r1`, `r2` descriptors of [`DmaDesc`], in its own
/// addresses; the kernel checks and translates it and starts the channel.
/// The descriptors the channel runs (a descriptor splits where its pages
/// aren't physically contiguous), or an error code.
pub const SYS_DMA: u32 = 10;

/// A DMA descriptor as a game writes it for [`SYS_DMA`]: its own source
/// and target addresses (a device side: an address in its granted window)
/// and the PXA27x `DCMD` word (no interrupts, fly-by or compare).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct DmaDesc {
    /// The source.
    pub src: u32,
    /// The target.
    pub dst: u32,
    /// `DCMD`.
    pub cmd: u32,
}

/// The first error code: results at or above it are errors.
pub const ERR: u32 = 0xFFFF_FF00;

/// Whether a call's result is an error code.
#[must_use]
pub const fn is_err(v: u32) -> bool {
    v >= ERR
}

#[cfg(target_os = "none")]
mod arch {
    /// One system call.
    #[inline(always)]
    pub fn syscall(n: u32, a0: u32, a1: u32, a2: u32, a3: u32) -> u32 {
        let r: u32;
        // SAFETY: `SWI` enters the kernel, which reads r0-r3 and r7 and
        // writes r0; it touches no other register or memory of ours.
        unsafe {
            core::arch::asm!(
                "swi #0",
                inlateout("r0") a0 => r,
                in("r1") a1,
                in("r2") a2,
                in("r3") a3,
                in("r7") n,
                options(nostack),
            );
        }
        r
    }
}

#[cfg(not(target_os = "none"))]
mod arch {
    /// Host builds (tests, lints) have no kernel.
    pub fn syscall(_: u32, _: u32, _: u32, _: u32, _: u32) -> u32 {
        super::ERR | 0xFF
    }
}

/// Calls the port behind `handle` with three arguments.
#[must_use]
pub fn call(handle: u32, a: u32, b: u32, c: u32) -> u32 {
    arch::syscall(SYS_CALL, handle, a, b, c)
}

/// Answers the call this activation serves; never comes back.
pub fn ret(v: u32) -> ! {
    arch::syscall(SYS_RETURN, v, 0, 0, 0);
    // Not in a call: nothing to return to.
    exit(ERR | 5)
}

/// Waits for the granted interrupt `line`; the pin's level after it fired,
/// or an error code.
#[must_use]
pub fn irq_wait(line: u32) -> u32 {
    arch::syscall(SYS_IRQ_WAIT, line, 0, 0, 0)
}

/// A test program's report; the thread ends.
pub fn report(a: u32, b: u32, c: u32, d: u32) -> ! {
    arch::syscall(SYS_REPORT, a, b, c, d);
    exit(ERR | 8)
}

/// A cache operation on this process's own memory ([`SYS_CACHE`]); 0 or an
/// error code.
#[must_use]
pub fn cache(op: u32, addr: u32, len: u32) -> u32 {
    arch::syscall(SYS_CACHE, op, addr, len, 0)
}

/// Starts DMA channel `channel` on `chain` ([`SYS_DMA`]): the descriptors
/// the channel runs, or an error code.
#[must_use]
pub fn dma(channel: u32, chain: &[DmaDesc]) -> u32 {
    arch::syscall(
        SYS_DMA,
        channel,
        chain.as_ptr() as usize as u32,
        chain.len() as u32,
        0,
    )
}

/// Ends the process.
pub fn exit(code: u32) -> ! {
    arch::syscall(SYS_EXIT, code, 0, 0, 0);
    #[allow(clippy::empty_loop)]
    loop {}
}

/// Where the kernel maps granted device windows in a process (its own
/// addresses), one 4 KiB page per device in [`Device`] order.
pub const GRANT_VA: usize = 0x0060_0000;

/// The devices a board can grant a process (the kernel's grant table says
/// which process gets which).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Device {
    /// PXA27x PWM0 (the X51v's backlight).
    Pwm0 = 0,
    /// The OS timer's registers, read only (a game's clock: `OSCR0` at
    /// offset 0x10, 3.25 MHz).
    Ost = 1,
    /// The DMA controller's first page, read only (a game reads its
    /// channels' `DCSR`; it starts them through [`SYS_DMA`]).
    DmaStatus = 2,
}

impl Device {
    /// Where its window is in a process it's granted to.
    #[must_use]
    pub const fn va(self) -> usize {
        GRANT_VA + self as usize * 4096
    }
}

/// A device's registers, mapped into this process by the kernel at the
/// fixed address of its grant. In a process the device isn't granted to,
/// the page is unmapped and an access faults: no other memory can be
/// reached through a window.
pub struct Window {
    base: usize,
}

impl Window {
    /// The window of `device`.
    #[must_use]
    pub const fn of(device: Device) -> Self {
        Self { base: device.va() }
    }

    fn at(&self, offset: usize) -> *mut u32 {
        assert!(
            offset.is_multiple_of(4) && offset < 4096,
            "outside the device window"
        );
        (self.base + offset) as *mut u32
    }
}

impl RegisterBus for Window {
    fn read32(&mut self, offset: usize) -> u32 {
        // SAFETY: inside the grant page (`at` checks), aligned; the page is
        // the device's registers (uncached) or unmapped (a fault, which the
        // kernel turns into a restart): never ordinary memory.
        unsafe { self.at(offset).read_volatile() }
    }
    fn write32(&mut self, offset: usize, value: u32) {
        // SAFETY: as above.
        unsafe { self.at(offset).write_volatile(value) }
    }
}

/// Where Game Mode's large region starts in the game's process (G4,
/// `rp_kernel_mm::layout::LARGE_BASE`; the kernel checks they agree).
pub const LARGE_BASE: u32 = 0x7800_0000;

/// The game's part of Game Mode's large region (G7): `len` bytes from
/// [`LARGE_BASE`] the kernel mapped in 1 MiB sections, read and written a
/// word at a time by offset. A word outside the mapped part faults (the
/// kernel ends the game): nothing else can be reached through it.
pub struct Large {
    len: usize,
}

impl Large {
    /// The first `len` bytes of the region (what the kernel mapped).
    #[must_use]
    pub const fn new(len: usize) -> Self {
        Self { len }
    }

    /// The address of byte `offset` (for a DMA descriptor or a cache call).
    #[must_use]
    pub const fn addr(&self, offset: usize) -> u32 {
        LARGE_BASE + offset as u32
    }

    fn at(&self, offset: usize) -> *mut u32 {
        assert!(
            offset.is_multiple_of(4) && offset < self.len,
            "outside the large region"
        );
        (LARGE_BASE as usize + offset) as *mut u32
    }

    /// The word at `offset`.
    #[must_use]
    pub fn read(&self, offset: usize) -> u32 {
        // SAFETY: inside the mapped part (`at` checks), aligned; memory
        // only this process maps and no Rust value lives in (it is outside
        // the image), and a device may have written it: volatile.
        unsafe { self.at(offset).read_volatile() }
    }

    /// Writes the word at `offset`.
    pub fn write(&mut self, offset: usize, value: u32) {
        // SAFETY: as in `read`.
        unsafe { self.at(offset).write_volatile(value) }
    }
}

/// Names the program's entry: `fn(r0, r1, r2, r3) -> !` (see the crate
/// docs for what the kernel passes).
#[macro_export]
macro_rules! entry {
    ($f:path) => {
        #[unsafe(no_mangle)]
        extern "C" fn rp_user_entry(r0: u32, r1: u32, r2: u32, r3: u32) -> ! {
            $f(r0, r1, r2, r3)
        }
    };
}

#[cfg(all(target_os = "none", feature = "runtime"))]
core::arch::global_asm!(
    r#"
    .section .text._start, "ax"
    .arm
    .global _start
    @ The kernel gives us sp and r0-r3; everything else is zero.
_start:
    b       rp_user_entry
"#
);

/// A program fault: the kernel restarts a supervised host, or stops the
/// test. Never unwinds (panic = abort).
#[cfg(all(target_os = "none", feature = "runtime"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // The first page is never mapped in a process: a clean, attributable
    // fault (not address 0 itself, which the compiler may assume unused).
    // SAFETY: the read faults; the kernel never resumes it.
    unsafe { (0x10 as *const u32).read_volatile() };
    exit(ERR | 0xEE)
}
