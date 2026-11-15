//! The C boundary: what a C library asks of the system under it, as one
//! safe Rust trait ([`Sys`]).
//!
//! Iterate, don't imitate (rule 0). Windows Mobile puts coredll over kernel
//! traps; Linux puts glibc or musl over a few hundred system calls; an
//! embedded newlib port is a file of C stubs (`_write`, `_sbrk`) that trust
//! every pointer. Ours: picolibc needs nine calls, and each is a
//! `no_mangle` function here that turns the C program's pointers into
//! slices once and hands them to a [`Sys`] written in safe Rust above (the
//! port layer, over the card's files and the audio service). The port
//! layer's own C functions (`ports/include/retropane.h`: the clock first)
//! cross here too. All the
//! `unsafe` a C program brings into the OS is in this file (the layering
//! rule: only `hal/*` may hold any), and what answers the calls is checked
//! code with tests.
//!
//! The memory routines (`memcpy`, `memset`, ...) a C program calls are
//! `rp-hal-mem`'s, the same tuned ones Rust code uses; picolibc's own are
//! left out of the build.

#![cfg_attr(not(test), no_std)]

/// Why a call failed: the C library's `errno` values (picolibc's
/// `sys/errno.h`, the numbers C programs test against).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Errno {
    /// No such file or folder (`ENOENT`).
    NoEntry = 2,
    /// The card failed (`EIO`).
    Io = 5,
    /// Not an open handle (`EBADF`).
    BadHandle = 9,
    /// No memory, or no room (`ENOMEM`).
    NoMemory = 12,
    /// In use already (`EBUSY`).
    Busy = 16,
    /// A bad argument (`EINVAL`).
    Invalid = 22,
    /// Every handle is in use (`EMFILE`).
    NoHandle = 24,
    /// No room on the card (`ENOSPC`).
    NoSpace = 28,
    /// Not something this system does (`ENOSYS`).
    NoSys = 88,
}

/// `open`'s flags as picolibc passes them (`sys/_default_fcntl.h`).
pub mod flags {
    /// The access mode's bits: 0 read, 1 write, 2 both.
    pub const ACCESS: i32 = 3;
    /// Every write goes to the end (`O_APPEND`).
    pub const APPEND: i32 = 0x0400;
    /// Make the file if it is not there (`O_CREAT`).
    pub const CREATE: i32 = 0x0040;
    /// Empty the file first (`O_TRUNC`).
    pub const TRUNCATE: i32 = 0x0200;
}

/// One input event as a C program reads it (`struct rp_event` in
/// `ports/include/retropane.h`: the same 16 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Event {
    /// [`event::KEY`] or [`event::PEN`].
    pub kind: u8,
    /// A key's code (`event::KEY_*`), or what the stylus did
    /// (`event::PEN_*`).
    pub code: u8,
    /// A key: 1 pressed, 0 released. The stylus: 1 while it touches.
    pub down: u8,
    /// Zero.
    pub pad: u8,
    /// The stylus: where, in the game's screen pixels (0 for a key).
    pub x: i16,
    /// See [`Self::x`].
    pub y: i16,
    /// When it was read, on the program's clock ([`Sys::now_us`]).
    pub time_us: u64,
}

impl Event {
    /// No event (all zero).
    pub const NONE: Self = Self {
        kind: 0,
        code: 0,
        down: 0,
        pad: 0,
        x: 0,
        y: 0,
        time_us: 0,
    };

    /// A key pressed or released.
    #[must_use]
    pub const fn key(code: u8, down: bool, time_us: u64) -> Self {
        Self {
            kind: event::KEY,
            code,
            down: down as u8,
            pad: 0,
            x: 0,
            y: 0,
            time_us,
        }
    }

    /// The stylus landing, moving or lifting at `(x, y)`.
    #[must_use]
    pub const fn pen(code: u8, x: i16, y: i16, time_us: u64) -> Self {
        Self {
            kind: event::PEN,
            code,
            down: (code != event::PEN_UP) as u8,
            pad: 0,
            x,
            y,
            time_us,
        }
    }
}

/// The numbers in an [`Event`] and for `rp_input_open` (`RP_EV_*`,
/// `RP_KEY_*`, `RP_PEN_*`, `RP_TURN_*` in `retropane.h`).
pub mod event {
    /// A button.
    pub const KEY: u8 = 1;
    /// The stylus.
    pub const PEN: u8 = 2;

    /// The D-pad, as the player holds the device.
    pub const KEY_UP: u8 = 1;
    /// See [`KEY_UP`].
    pub const KEY_DOWN: u8 = 2;
    /// See [`KEY_UP`].
    pub const KEY_LEFT: u8 = 3;
    /// See [`KEY_UP`].
    pub const KEY_RIGHT: u8 = 4;
    /// The D-pad's centre.
    pub const KEY_ENTER: u8 = 5;
    /// The four program buttons, by what is printed on them.
    pub const KEY_CALENDAR: u8 = 6;
    /// See [`KEY_CALENDAR`].
    pub const KEY_CONTACTS: u8 = 7;
    /// See [`KEY_CALENDAR`].
    pub const KEY_MAIL: u8 = 8;
    /// See [`KEY_CALENDAR`].
    pub const KEY_HOME: u8 = 9;
    /// The side button.
    pub const KEY_RECORD: u8 = 10;
    /// The wireless button.
    pub const KEY_WIFI: u8 = 11;

    /// The stylus landed.
    pub const PEN_DOWN: u8 = 1;
    /// It moved while down.
    pub const PEN_MOVE: u8 = 2;
    /// It lifted (where it last was).
    pub const PEN_UP: u8 = 3;

    /// The screen upright (480 x 640).
    pub const TURN_NONE: u8 = 0;
    /// Landscape, the device's top at the player's left.
    pub const TURN_LEFT: u8 = 1;
    /// Landscape, the device's top at the player's right.
    pub const TURN_RIGHT: u8 = 2;
}

/// What the system under a C program does for it. One object answers every
/// call; the image registers it with `start` before any C code runs.
pub trait Sys {
    /// Writes `bytes` to handle `fd` (1 and 2 are the console).
    ///
    /// # Errors
    /// The handle is not open for writing, or the write failed.
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<usize, Errno>;
    /// Reads into `into` from handle `fd`; 0 at the end of the file.
    ///
    /// # Errors
    /// The handle is not open for reading, or the read failed.
    fn read(&mut self, fd: i32, into: &mut [u8]) -> Result<usize, Errno>;
    /// Opens the file named `path` ([`flags`]); the new handle, 3 or more.
    ///
    /// # Errors
    /// No such file, no free handle, or the card failed.
    fn open(&mut self, path: &[u8], flags: i32) -> Result<i32, Errno>;
    /// Closes handle `fd`.
    ///
    /// # Errors
    /// Not an open handle, or a written file could not be put on the card.
    fn close(&mut self, fd: i32) -> Result<(), Errno>;
    /// Moves handle `fd`'s place: `whence` 0 from the start, 1 from here,
    /// 2 from the end. The new place from the start.
    ///
    /// # Errors
    /// Not an open file, or a place before its start.
    fn seek(&mut self, fd: i32, by: i64, whence: i32) -> Result<i64, Errno>;
    /// Removes the file named `path`.
    ///
    /// # Errors
    /// No such file, or the card failed.
    fn remove(&mut self, path: &[u8]) -> Result<(), Errno>;
    /// Moves the end of the C heap by `by` bytes (`sbrk`): the old end, or
    /// `None` when the room is used up. [`Break`] does this.
    fn grow(&mut self, by: isize) -> Option<usize>;
    /// Microseconds since the program's system was started; never goes
    /// back. (The date is not known to the port layer yet: `time()` counts
    /// from 1970 as if the program had started then.)
    fn now_us(&mut self) -> u64;
    /// Opens the program's sound (`rp_snd_open`): `rate` in Hz, 1 or 2
    /// channels of 16-bit samples.
    ///
    /// # Errors
    /// No sound here (`ENOSYS`, the default), open already, or a rate or
    /// channel count that is not taken.
    fn snd_open(&mut self, _rate: u32, _channels: u8) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// Frames that may be written now (`rp_snd_room`).
    ///
    /// # Errors
    /// The sound is not open.
    fn snd_room(&mut self) -> Result<u32, Errno> {
        Err(Errno::NoSys)
    }
    /// Takes interleaved samples (`rp_snd_write`): as many whole frames as
    /// there is room for; how many frames.
    ///
    /// # Errors
    /// The sound is not open.
    fn snd_write(&mut self, _samples: &[i16]) -> Result<u32, Errno> {
        Err(Errno::NoSys)
    }
    /// The program's frames that have really been played
    /// (`rp_snd_played`).
    ///
    /// # Errors
    /// The sound is not open.
    fn snd_played(&mut self) -> Result<u64, Errno> {
        Err(Errno::NoSys)
    }
    /// Closes the sound (`rp_snd_close`).
    ///
    /// # Errors
    /// The sound is not open.
    fn snd_close(&mut self) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// Starts the program's input (`rp_input_open`), the screen held as
    /// `turn` (`event::TURN_*`).
    ///
    /// # Errors
    /// No input here (`ENOSYS`, the default), open already, or no such
    /// turn.
    fn input_open(&mut self, _turn: u8) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// The oldest input event not yet given (`rp_input_poll`).
    ///
    /// # Errors
    /// The input is not open.
    fn input_poll(&mut self) -> Result<Option<Event>, Errno> {
        Err(Errno::NoSys)
    }
    /// The program is over (it returned, or called `exit`): stop what it
    /// left running (its sound). Nothing by default.
    fn end(&mut self) {}
}

/// The C heap's room: one run of RAM whose used part grows from its start
/// (`sbrk`'s model; picolibc's `malloc` asks for more as it needs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Break {
    start: usize,
    end: usize,
    at: usize,
}

impl Break {
    /// A heap of the addresses `start..end`, none of it used.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end: if end < start { start } else { end },
            at: start,
        }
    }

    /// Moves the end of the used part by `by` and returns where it was;
    /// `None`, and nothing moved, if that leaves the room at either end.
    pub fn grow(&mut self, by: isize) -> Option<usize> {
        let old = self.at;
        let new = old.checked_add_signed(by)?;
        if new < self.start || new > self.end {
            return None;
        }
        self.at = new;
        Some(old)
    }

    /// Bytes in use.
    #[must_use]
    pub const fn used(&self) -> usize {
        self.at - self.start
    }
}

/// Microseconds as `gettimeofday`'s whole seconds and the microseconds
/// over (under a million).
#[must_use]
pub const fn split_us(us: u64) -> (i64, i32) {
    ((us / 1_000_000) as i64, (us % 1_000_000) as i32)
}

#[cfg(target_os = "none")]
mod face {
    use super::{Errno, Event, Sys};
    use core::ffi::{CStr, c_char, c_int, c_void};
    use rp_hal_pxa27x::rt::Global;

    /// The registered system. It may hold a device (the console's UART).
    struct Held(&'static mut dyn Sys);
    // SAFETY: the X51v has one core and a C program one thread; the object
    // is only ever reached through `SYS`'s lock, which masks interrupts and
    // refuses re-entry, so it never moves between contexts.
    unsafe impl Send for Held {}

    /// The object that answers the C library's calls.
    static SYS: Global<Option<Held>> = Global::new(None);
    /// What `_exit` does: taken out of its own lock before it runs, so it
    /// may itself print.
    static EXIT: Global<Option<fn(i32) -> !>> = Global::new(None);

    unsafe extern "C" {
        /// picolibc's `errno` (one global: the build has `__GLOBAL_ERRNO`).
        static mut errno: c_int;
        /// picolibc's: runs the constructors of the program's global
        /// objects (boot/link.x's `.init_array`).
        fn __libc_init_array();
        /// The C program.
        fn main() -> c_int;
        /// picolibc's `fflush`.
        fn fflush(stream: *mut c_void) -> c_int;
        /// picolibc's standard output stream.
        static stdout: *mut c_void;
    }

    /// Registers what answers the C library's calls and what `_exit` does.
    /// Before any C code runs.
    pub fn start(sys: &'static mut dyn Sys, exit: fn(i32) -> !) {
        SYS.with(|s| *s = Some(Held(sys)));
        EXIT.with(|e| *e = Some(exit));
    }

    /// Runs the C program's `main` and returns its result, with what it
    /// printed pushed out.
    #[must_use]
    pub fn run() -> i32 {
        // SAFETY: picolibc's own walk of the link script's constructor
        // table, once, before the program; `start` has registered the
        // system a constructor may call into.
        unsafe { __libc_init_array() };
        // SAFETY: `main` is the linked C program's entry, taking nothing;
        // `start` has registered the system it calls into.
        let code = unsafe { main() };
        // SAFETY: `stdout` is picolibc's own stream pointer, read as the
        // plain word it is and given back to picolibc's `fflush`.
        unsafe {
            let _ = fflush(core::ptr::addr_of!(stdout).read());
        }
        end();
        code
    }

    /// Tells the system the program is over ([`Sys::end`]).
    fn end() {
        let _ = with(|s| {
            s.end();
            Ok(())
        });
    }

    fn fail(e: Errno) -> isize {
        // SAFETY: one core and no C thread: nothing else holds `errno`
        // while this plain word is written.
        unsafe { core::ptr::addr_of_mut!(errno).write(e as c_int) };
        -1
    }

    fn with<R>(f: impl FnOnce(&mut dyn Sys) -> Result<R, Errno>) -> Result<R, Errno> {
        SYS.with(|s| match s {
            Some(held) => f(&mut *held.0),
            None => Err(Errno::NoSys),
        })
    }

    /// `write(2)`.
    ///
    /// # Safety
    /// `buf` points at `n` readable bytes (the C library's contract).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn write(fd: c_int, buf: *const u8, n: usize) -> isize {
        if buf.is_null() || n > isize::MAX as usize {
            return fail(Errno::Invalid);
        }
        // SAFETY: the caller's contract: `n` readable bytes at `buf`,
        // not null (checked), not changed while this call runs (one core).
        let bytes = unsafe { core::slice::from_raw_parts(buf, n) };
        match with(|s| s.write(fd, bytes)) {
            Ok(done) => done as isize,
            Err(e) => fail(e),
        }
    }

    /// `read(2)`.
    ///
    /// # Safety
    /// `buf` points at `n` writable bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn read(fd: c_int, buf: *mut u8, n: usize) -> isize {
        if buf.is_null() || n > isize::MAX as usize {
            return fail(Errno::Invalid);
        }
        // SAFETY: the caller's contract: `n` writable bytes at `buf`, not
        // null (checked), the C program's alone while this call runs.
        let into = unsafe { core::slice::from_raw_parts_mut(buf, n) };
        match with(|s| s.read(fd, into)) {
            Ok(done) => done as isize,
            Err(e) => fail(e),
        }
    }

    /// `open(2)`; the mode a third argument would carry is not used (the
    /// card's file systems have no permissions).
    ///
    /// # Safety
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn open(path: *const c_char, flags: c_int) -> c_int {
        if path.is_null() {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: a NUL-terminated string at `path`.
        let name = unsafe { CStr::from_ptr(path) }.to_bytes();
        match with(|s| s.open(name, flags)) {
            Ok(fd) => fd,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `close(2)`.
    #[unsafe(no_mangle)]
    pub extern "C" fn close(fd: c_int) -> c_int {
        match with(|s| s.close(fd)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `lseek(2)`; picolibc's `off_t` is 64 bits.
    #[unsafe(no_mangle)]
    pub extern "C" fn lseek(fd: c_int, by: i64, whence: c_int) -> i64 {
        match with(|s| s.seek(fd, by, whence)) {
            Ok(at) => at,
            Err(e) => fail(e) as i64,
        }
    }

    /// `unlink(2)`.
    ///
    /// # Safety
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
        if path.is_null() {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: a NUL-terminated string at `path`.
        let name = unsafe { CStr::from_ptr(path) }.to_bytes();
        match with(|s| s.remove(name)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `sbrk(2)`: the heap's old end, or -1 with `ENOMEM`.
    #[unsafe(no_mangle)]
    pub extern "C" fn sbrk(by: isize) -> *mut c_void {
        match with(|s| s.grow(by).ok_or(Errno::NoMemory)) {
            Ok(old) => old as *mut c_void,
            Err(e) => fail(e) as usize as *mut c_void,
        }
    }

    /// The port layer's clock (`ports/include/retropane.h`): microseconds
    /// since the program's system was started, 0 before one is registered.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_time_us() -> u64 {
        with(|s| Ok(s.now_us())).unwrap_or(0)
    }

    /// `rp_snd_open` (`ports/include/retropane.h`): 0, or -1 with `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_snd_open(rate: u32, channels: c_int) -> c_int {
        let Ok(channels) = u8::try_from(channels) else {
            return fail(Errno::Invalid) as c_int;
        };
        match with(|s| s.snd_open(rate, channels)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_snd_room`: frames that may be written now, or -1 with `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_snd_room() -> c_int {
        match with(|s| s.snd_room()) {
            Ok(frames) => c_int::try_from(frames).unwrap_or(c_int::MAX),
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_snd_write`: the frames taken of `count` 16-bit samples
    /// (interleaved), or -1 with `errno`.
    ///
    /// # Safety
    /// `samples` points at `count` readable 16-bit samples.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rp_snd_write(samples: *const i16, count: usize) -> c_int {
        if samples.is_null() || !samples.is_aligned() || count > isize::MAX as usize / 2 {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: `count` readable samples at
        // `samples`, not null and aligned for 16 bits (both checked), not
        // changed while this call runs (one core, one C thread).
        let samples = unsafe { core::slice::from_raw_parts(samples, count) };
        match with(|s| s.snd_write(samples)) {
            Ok(frames) => c_int::try_from(frames).unwrap_or(c_int::MAX),
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_snd_played`: the program's frames really played, or -1 with
    /// `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_snd_played() -> i64 {
        match with(|s| s.snd_played()) {
            Ok(frames) => i64::try_from(frames).unwrap_or(i64::MAX),
            Err(e) => fail(e) as i64,
        }
    }

    /// `rp_snd_close`: 0, or -1 with `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_snd_close() -> c_int {
        match with(|s| s.snd_close()) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_input_open` (`ports/include/retropane.h`): 0, or -1 with
    /// `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_input_open(turn: c_int) -> c_int {
        let Ok(turn) = u8::try_from(turn) else {
            return fail(Errno::Invalid) as c_int;
        };
        match with(|s| s.input_open(turn)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_input_poll`: 1 with the oldest event in `*ev`, 0 when there is
    /// none (`*ev` untouched), or -1 with `errno`.
    ///
    /// # Safety
    /// `ev` points at a writable `struct rp_event`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rp_input_poll(ev: *mut Event) -> c_int {
        if ev.is_null() || !ev.is_aligned() {
            return fail(Errno::Invalid) as c_int;
        }
        match with(|s| s.input_poll()) {
            Ok(Some(e)) => {
                // SAFETY: the caller's contract: a writable `struct
                // rp_event` at `ev`, not null and aligned (both checked),
                // written whole.
                unsafe { ev.write(e) };
                1
            }
            Ok(None) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// picolibc's `struct timeval` (`sys/_timeval.h`: a 64-bit `time_t`,
    /// a 32-bit `suseconds_t`).
    #[repr(C)]
    pub struct Timeval {
        sec: i64,
        usec: i32,
    }

    /// `gettimeofday(2)`: [`rp_time_us`] as seconds and microseconds (the
    /// C library's `time()` reads it). The time zone is not filled in.
    ///
    /// # Safety
    /// `tv` is null or points at a writable `struct timeval`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn gettimeofday(tv: *mut Timeval, _tz: *mut c_void) -> c_int {
        if tv.is_null() {
            return 0;
        }
        match with(|s| Ok(s.now_us())) {
            Ok(us) => {
                let (sec, usec) = super::split_us(us);
                // SAFETY: the caller's contract: a writable `struct
                // timeval` at `tv`, not null (checked), written whole.
                unsafe { tv.write(Timeval { sec, usec }) };
                0
            }
            Err(e) => fail(e) as c_int,
        }
    }

    /// `raise(3)`: there are no signals here. The C library calls it from
    /// `abort()` (a failed `assert`, C++'s pure-virtual trap, `new` out of
    /// memory): the image's end, as a failure (128 plus the signal, the
    /// number a shell would show).
    #[unsafe(no_mangle)]
    pub extern "C" fn raise(signal: c_int) -> c_int {
        _exit(128 + (signal & 0x7F))
    }

    /// `_exit(2)`: the image's own end.
    #[unsafe(no_mangle)]
    pub extern "C" fn _exit(code: c_int) -> ! {
        end();
        match EXIT.with(|e| *e) {
            Some(exit) => exit(code),
            None => panic!("a C program ended before the system was registered"),
        }
    }
}

#[cfg(target_os = "none")]
pub use face::{run, start};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heap_grows_inside_its_room_and_no_further() {
        let mut b = Break::new(0x1000, 0x1100);
        assert_eq!(b.grow(0), Some(0x1000));
        assert_eq!(b.grow(0x80), Some(0x1000));
        assert_eq!(b.grow(0x80), Some(0x1080));
        assert_eq!(b.used(), 0x100);
        // One byte more than the room: refused, and nothing moved.
        assert_eq!(b.grow(1), None);
        assert_eq!(b.grow(0), Some(0x1100));
        // Given back; past its start refused.
        assert_eq!(b.grow(-0x100), Some(0x1100));
        assert_eq!(b.grow(-1), None);
        assert_eq!(b.used(), 0);
        // An amount that would wrap the address is refused.
        assert_eq!(b.grow(isize::MAX), None);
        assert_eq!(b.grow(isize::MIN), None);
    }

    #[test]
    fn a_room_given_backwards_is_empty() {
        let mut b = Break::new(0x2000, 0x1000);
        assert_eq!(b.grow(1), None);
        assert_eq!(b.grow(0), Some(0x2000));
    }

    #[test]
    fn microseconds_split_into_seconds_and_the_rest() {
        assert_eq!(split_us(0), (0, 0));
        assert_eq!(split_us(999_999), (0, 999_999));
        assert_eq!(split_us(1_000_000), (1, 0));
        assert_eq!(split_us(86_400_000_001), (86_400, 1));
        // The largest count: seconds that fit, the rest under a million.
        let (s, u) = split_us(u64::MAX);
        assert_eq!(s as u64 * 1_000_000 + u as u64, u64::MAX);
        assert!((0..1_000_000).contains(&u));
    }

    #[test]
    fn an_event_is_the_c_struct_s_sixteen_bytes() {
        // `struct rp_event` (retropane.h): four bytes, two 16-bit
        // coordinates, the 64-bit time on its own 8-byte boundary.
        assert_eq!(core::mem::size_of::<Event>(), 16);
        assert_eq!(core::mem::align_of::<Event>(), 8);
        assert_eq!(core::mem::offset_of!(Event, kind), 0);
        assert_eq!(core::mem::offset_of!(Event, code), 1);
        assert_eq!(core::mem::offset_of!(Event, down), 2);
        assert_eq!(core::mem::offset_of!(Event, x), 4);
        assert_eq!(core::mem::offset_of!(Event, y), 6);
        assert_eq!(core::mem::offset_of!(Event, time_us), 8);
        let k = Event::key(event::KEY_HOME, true, 7);
        assert_eq!(
            (k.kind, k.code, k.down, k.x, k.y, k.time_us),
            (1, 9, 1, 0, 0, 7)
        );
        // The stylus is down while it lands and moves, up when it lifts.
        assert_eq!(Event::pen(event::PEN_DOWN, -1, 479, 0).down, 1);
        assert_eq!(Event::pen(event::PEN_MOVE, 639, 0, 0).down, 1);
        let up = Event::pen(event::PEN_UP, 3, 4, 9);
        assert_eq!((up.kind, up.down, up.x, up.y, up.pad), (2, 0, 3, 4, 0));
        assert_eq!(Event::NONE.kind, 0);
    }

    #[test]
    fn the_error_numbers_are_the_c_librarys() {
        // picolibc 1.8.12's sys/errno.h.
        assert_eq!(Errno::NoEntry as i32, 2);
        assert_eq!(Errno::Io as i32, 5);
        assert_eq!(Errno::BadHandle as i32, 9);
        assert_eq!(Errno::NoMemory as i32, 12);
        assert_eq!(Errno::Busy as i32, 16);
        assert_eq!(Errno::Invalid as i32, 22);
        assert_eq!(Errno::NoHandle as i32, 24);
        assert_eq!(Errno::NoSpace as i32, 28);
        assert_eq!(Errno::NoSys as i32, 88);
        // Its sys/_default_fcntl.h: _FAPPEND, _FCREAT, _FTRUNC.
        assert_eq!(
            (flags::APPEND, flags::CREATE, flags::TRUNCATE),
            (0x0400, 0x0040, 0x0200)
        );
    }
}
