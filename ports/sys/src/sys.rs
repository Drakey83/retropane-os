//! What answers a C program's library calls ([`rp_hal_cabi::Sys`]): the
//! console on handles 0 to 2, the card's files ([`Files`]) from 3 up, the
//! C heap and the clock ([`Micros`]).
//!
//! Iterate, don't imitate (rule 0). A C library's files on Windows Mobile
//! and Linux are the kernel's, with its cache and its permissions; a
//! console port's are the vendor SDK's. Here `fopen` reaches the card's FAT
//! or exFAT code in three calls (picolibc's stream, `hal/cabi`, this file),
//! with no kernel between and no second buffer: the C library's own stream
//! buffer is the only copy. A file opened for writing is the whole-file
//! write of [`Files`] (a save is on the card complete or not at all), which
//! is what `fopen(.., "wb")` asks for; append and read-write opens, which
//! that cannot honour, are refused with `ENOSYS` instead of half-working.

use rp_fat::Error as FatError;
use rp_hal_api::BlockDevice;
use rp_hal_cabi::{Break, Errno, Sys, flags};

use crate::clock::Micros;
use crate::files::{Error, Fd, Files, Whence};
use crate::sound::Voice;

/// Where what a C program prints goes (the serial line, the screen's
/// console, a log on the card).
pub trait Console {
    /// Shows `bytes` (text; a line ends with `\n`).
    fn put(&mut self, bytes: &[u8]);
}

/// The first handle that is a file: 0, 1 and 2 are the console.
const FIRST: i32 = 3;

/// A C program's system: `N` file handles over the card, a console, a heap.
pub struct System<'a, D: BlockDevice, const N: usize, C: Console> {
    files: Files<'a, D, N>,
    console: C,
    heap: Break,
    /// Reads the OS timer's counter (3.25 MHz).
    ticks: fn() -> u32,
    clock: Micros,
    /// The game's sound, where the image has it.
    sound: Option<&'a mut dyn Voice>,
}

/// A [`Files`] error as the C library's `errno`.
#[must_use]
pub fn errno(e: Error) -> Errno {
    match e {
        Error::NotFound => Errno::NoEntry,
        Error::BadPath => Errno::Invalid,
        Error::BadHandle => Errno::BadHandle,
        Error::NoHandle => Errno::NoHandle,
        Error::NoRoom | Error::Card(FatError::NoRoom) => Errno::NoSpace,
        Error::Card(_) => Errno::Io,
    }
}

/// A C handle as the table's, if it is a file's.
fn file(fd: i32) -> Result<Fd, Errno> {
    fd.checked_sub(FIRST)
        .and_then(|f| Fd::try_from(f).ok())
        .ok_or(Errno::BadHandle)
}

impl<'a, D: BlockDevice, const N: usize, C: Console> System<'a, D, N, C> {
    /// The system over `files`, printing to `console`, its C heap in
    /// `heap`, its clock started now from `ticks` (the OS timer's counter).
    pub fn new(files: Files<'a, D, N>, console: C, heap: Break, ticks: fn() -> u32) -> Self {
        Self {
            files,
            console,
            heap,
            ticks,
            clock: Micros::starting_at(ticks()),
            sound: None,
        }
    }

    /// Gives the system its sound (the codec up, I2S running): what
    /// `rp_snd_open` and the calls after it reach. (Set in place: the
    /// system is too big to pass through a function by value, R250.)
    pub fn set_sound(&mut self, sound: &'a mut dyn Voice) {
        self.sound = Some(sound);
    }

    /// The sound and the time in milliseconds (as the mixer counts it),
    /// for a sound call.
    fn voice(&mut self) -> Result<(&mut dyn Voice, u32), Errno> {
        let now = (self.clock.at((self.ticks)()) / 1_000) as u32;
        match self.sound.as_mut() {
            Some(v) => Ok((&mut **v, now)),
            None => Err(Errno::NoSys),
        }
    }

    /// The file table (folders, and what C's library has no call for).
    pub fn files(&mut self) -> &mut Files<'a, D, N> {
        &mut self.files
    }

    /// Bytes of the C heap in use.
    #[must_use]
    pub fn heap_used(&self) -> usize {
        self.heap.used()
    }

    /// The parts back (the card handed on, the console for the last line).
    pub fn into_parts(self) -> (Files<'a, D, N>, C) {
        (self.files, self.console)
    }
}

impl<D: BlockDevice, const N: usize, C: Console> Sys for System<'_, D, N, C> {
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<usize, Errno> {
        match fd {
            1 | 2 => {
                self.console.put(bytes);
                Ok(bytes.len())
            }
            _ => self.files.write(file(fd)?, bytes).map_err(errno),
        }
    }

    fn read(&mut self, fd: i32, into: &mut [u8]) -> Result<usize, Errno> {
        match fd {
            // Nothing is ever typed: the end of the input at once.
            0 => Ok(0),
            _ => self.files.read(file(fd)?, into).map_err(errno),
        }
    }

    fn open(&mut self, path: &[u8], how: i32) -> Result<i32, Errno> {
        let fd = match how & flags::ACCESS {
            0 if how & (flags::CREATE | flags::TRUNCATE | flags::APPEND) == 0 => {
                self.files.open(path)
            }
            // "w": write only, made new, whatever was there replaced.
            1 if how & flags::APPEND == 0 && how & flags::TRUNCATE != 0 => self.files.create(path),
            _ => return Err(Errno::NoSys),
        }
        .map_err(errno)?;
        Ok(i32::from(fd) + FIRST)
    }

    fn close(&mut self, fd: i32) -> Result<(), Errno> {
        self.files.close(file(fd)?).map_err(errno)
    }

    fn seek(&mut self, fd: i32, by: i64, whence: i32) -> Result<i64, Errno> {
        let whence = match whence {
            0 => Whence::Start,
            1 => Whence::Here,
            2 => Whence::End,
            _ => return Err(Errno::Invalid),
        };
        let at = self
            .files
            .seek(file(fd)?, by, whence)
            .map_err(|e| match e {
                // A place before the start is a bad argument, not a bad card.
                Error::BadPath => Errno::Invalid,
                e => errno(e),
            })?;
        i64::try_from(at).map_err(|_| Errno::Invalid)
    }

    fn remove(&mut self, path: &[u8]) -> Result<(), Errno> {
        self.files.remove(path).map_err(errno)
    }

    fn grow(&mut self, by: isize) -> Option<usize> {
        self.heap.grow(by)
    }

    fn now_us(&mut self) -> u64 {
        let us = self.clock.at((self.ticks)());
        // A game reads the clock every frame: the mixer's wake, if due.
        if let Some(v) = self.sound.as_mut() {
            v.pump((us / 1_000) as u32);
        }
        us
    }

    fn snd_open(&mut self, rate: u32, channels: u8) -> Result<(), Errno> {
        let (v, now) = self.voice()?;
        v.open(rate, channels, now)
    }

    fn snd_room(&mut self) -> Result<u32, Errno> {
        let (v, now) = self.voice()?;
        v.room(now)
    }

    fn snd_write(&mut self, samples: &[i16]) -> Result<u32, Errno> {
        let (v, now) = self.voice()?;
        v.write(samples, now)
    }

    fn snd_played(&mut self) -> Result<u64, Errno> {
        let (v, now) = self.voice()?;
        v.played(now)
    }

    fn snd_close(&mut self) -> Result<(), Errno> {
        let (v, now) = self.voice()?;
        v.close(now)
    }

    fn end(&mut self) {
        if let Some(v) = self.sound.as_mut() {
            v.shut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::{Disk, each, pattern};

    /// What was printed.
    struct Said(Vec<u8>);

    impl Console for &mut Said {
        fn put(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
    }

    /// picolibc's `fopen` modes as the flags it passes (`__stdio_flags`).
    const R: i32 = 0;
    const W: i32 = 1 | flags::CREATE | flags::TRUNCATE;
    const A: i32 = 1 | flags::CREATE | flags::APPEND;
    const R_PLUS: i32 = 2;
    const W_PLUS: i32 = 2 | flags::CREATE | flags::TRUNCATE;

    #[test]
    fn a_c_program_writes_a_save_and_reads_it_back_through_its_handles() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 100_000][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 4> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0x1000, 0x3000), || 0);
            s.files().mkdir(b"valve").unwrap();
            // Nothing there yet.
            assert_eq!(s.open(b"valve/save.sav", R), Err(Errno::NoEntry), "{kind}");
            // Written in pieces, as a stream's buffer hands them over.
            let save = pattern(40_001, 9);
            let w = s.open(b"valve/save.sav", W).unwrap();
            assert_eq!(w, 3, "the first file handle");
            for piece in save.chunks(1024) {
                assert_eq!(s.write(w, piece), Ok(piece.len()));
            }
            assert_eq!(s.close(w), Ok(()));
            assert_eq!(s.close(w), Err(Errno::BadHandle), "closed once");
            // Read back: its size by a seek to the end, then from the start.
            let r = s.open(b"VALVE\\SAVE.SAV", R).unwrap();
            assert_eq!(s.seek(r, 0, 2), Ok(save.len() as i64));
            assert_eq!(s.seek(r, -1, 0), Err(Errno::Invalid), "before the start");
            assert_eq!(s.seek(r, 0, 3), Err(Errno::Invalid), "no such whence");
            assert_eq!(s.seek(r, 100, 0), Ok(100));
            assert_eq!(s.seek(r, -100, 1), Ok(0));
            let mut got = vec![0u8; save.len()];
            let mut n = 0;
            while n < got.len() {
                let k = s.read(r, &mut got[n..(n + 4096).min(save.len())]).unwrap();
                assert!(k > 0, "{kind}: short at {n}");
                n += k;
            }
            assert_eq!(got, save, "{kind}");
            assert_eq!(s.read(r, &mut [0u8; 8]), Ok(0), "its end");
            // A file open for reading is not written.
            assert_eq!(s.write(r, b"x"), Err(Errno::BadHandle));
            assert_eq!(s.close(r), Ok(()));
            // Removed, and then not there.
            assert_eq!(s.remove(b"valve/save.sav"), Ok(()));
            assert_eq!(s.remove(b"valve/save.sav"), Err(Errno::NoEntry));
            assert_eq!(s.open(b"valve/save.sav", R), Err(Errno::NoEntry));
        });
    }

    #[test]
    fn the_console_is_handles_one_and_two_and_nothing_is_typed() {
        each(|vol, _| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 1024][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 2> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0x1000, 0x1010), || 0);
            assert_eq!(s.write(1, b"out\n"), Ok(4));
            assert_eq!(s.write(2, b"err\n"), Ok(4));
            assert_eq!(s.read(0, &mut [0u8; 16]), Ok(0));
            // The console is not a file: no seek, no close, no read of it.
            for fd in [0, 1, 2] {
                assert_eq!(s.seek(fd, 0, 0), Err(Errno::BadHandle));
                assert_eq!(s.close(fd), Err(Errno::BadHandle));
            }
            assert_eq!(s.read(1, &mut [0u8; 4]), Err(Errno::BadHandle));
            assert_eq!(s.write(0, b"x"), Err(Errno::BadHandle));
            // Handles that are no handle at all.
            for fd in [-1, 3, 4, 5, i32::MAX, i32::MIN] {
                assert_eq!(s.write(fd, b"x"), Err(Errno::BadHandle), "{fd}");
                assert_eq!(s.read(fd, &mut [0u8; 4]), Err(Errno::BadHandle), "{fd}");
            }
            // The heap: its room and no more.
            assert_eq!(s.grow(0x10), Some(0x1000));
            assert_eq!(s.grow(1), None);
            assert_eq!(s.heap_used(), 0x10);
            let (_, _) = s.into_parts();
            assert_eq!(said.0, b"out\nerr\n");
        });
    }

    #[test]
    fn opens_that_cannot_be_honoured_are_refused_and_handles_run_out_cleanly() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 16][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 2> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0, 0), || 0);
            let w = s.open(b"a.txt", W).unwrap();
            assert_eq!(s.write(w, b"0123456789"), Ok(10));
            // More than the room for a written file: nothing taken.
            assert_eq!(s.write(w, b"0123456789"), Err(Errno::NoSpace), "{kind}");
            assert_eq!(s.close(w), Ok(()));
            // Append and read-write: not half-working, refused.
            for how in [A, R_PLUS, W_PLUS, R | flags::TRUNCATE] {
                assert_eq!(s.open(b"a.txt", how), Err(Errno::NoSys), "{how:#x}");
            }
            // Two handles, then none; one closed, one again.
            let a = s.open(b"a.txt", R).unwrap();
            let b = s.open(b"a.txt", R).unwrap();
            assert_eq!(s.open(b"a.txt", R), Err(Errno::NoHandle));
            assert_eq!(s.close(a), Ok(()));
            assert_eq!(s.open(b"a.txt", R), Ok(a));
            assert_eq!(s.close(b), Ok(()));
            // A name that is no path.
            assert_eq!(s.open(b"", R), Err(Errno::Invalid), "{kind}");
        });
    }

    #[test]
    fn the_clock_starts_at_the_system_s_making_and_follows_the_counter() {
        use std::sync::atomic::{AtomicU32, Ordering};
        // This test's alone (`each` runs its formats one after another).
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        each(|vol, kind| {
            COUNTER.store(u32::MAX - 12, Ordering::Relaxed);
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 16][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 2> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0, 0), || {
                COUNTER.load(Ordering::Relaxed)
            });
            assert_eq!(s.now_us(), 0, "{kind}: from its making, not zero");
            // 13 ticks on, through the counter's wrap: 4 microseconds.
            COUNTER.store(0, Ordering::Relaxed);
            assert_eq!(s.now_us(), 4);
            COUNTER.store(3_250_000, Ordering::Relaxed);
            assert_eq!(s.now_us(), 1_000_004);
            assert_eq!(s.now_us(), 1_000_004, "a counter that stood still");
        });
    }

    /// A voice that writes down what it was asked, and when.
    #[derive(Default)]
    struct Heard(Vec<(&'static str, u32)>);

    impl Voice for Heard {
        fn open(&mut self, rate: u32, channels: u8, now_ms: u32) -> Result<(), Errno> {
            self.0.push(("open", now_ms));
            if (rate, channels) == (22_050, 2) {
                Ok(())
            } else {
                Err(Errno::Invalid)
            }
        }
        fn room(&mut self, now_ms: u32) -> Result<u32, Errno> {
            self.0.push(("room", now_ms));
            Ok(77)
        }
        fn write(&mut self, samples: &[i16], now_ms: u32) -> Result<u32, Errno> {
            self.0.push(("write", now_ms));
            Ok(samples.len() as u32 / 2)
        }
        fn played(&mut self, now_ms: u32) -> Result<u64, Errno> {
            self.0.push(("played", now_ms));
            Ok(1 << 40)
        }
        fn close(&mut self, now_ms: u32) -> Result<(), Errno> {
            self.0.push(("close", now_ms));
            Err(Errno::BadHandle)
        }
        fn pump(&mut self, now_ms: u32) {
            self.0.push(("pump", now_ms));
        }
        fn shut(&mut self) {
            self.0.push(("shut", 0));
        }
    }

    #[test]
    fn sound_calls_reach_the_voice_with_the_clock_s_time_and_are_refused_without_one() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        each(|vol, kind| {
            COUNTER.store(0, Ordering::Relaxed);
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 16][..];
            let mut said = Said(Vec::new());
            // Without sound: not something this system does.
            let files: Files<'_, Disk, 2> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0, 0), || 0);
            assert_eq!(s.snd_open(44_100, 2), Err(Errno::NoSys), "{kind}");
            assert_eq!(s.snd_room(), Err(Errno::NoSys));
            assert_eq!(s.snd_write(&[0; 2]), Err(Errno::NoSys));
            assert_eq!(s.snd_played(), Err(Errno::NoSys));
            assert_eq!(s.snd_close(), Err(Errno::NoSys));
            assert_eq!(s.now_us(), 0);
            let (files, _) = s.into_parts();
            // With it: each call arrives with the clock's milliseconds, its
            // answer or its refusal comes back unchanged, and a read of the
            // clock is a chance for the mixer to wake.
            let mut heard = Heard::default();
            let mut s = System::new(files, &mut said, Break::new(0, 0), || {
                COUNTER.load(Ordering::Relaxed)
            });
            s.set_sound(&mut heard);
            let at = |ms: u32| COUNTER.store(ms * 3_250, Ordering::Relaxed);
            assert_eq!(s.snd_open(8_000, 1), Err(Errno::Invalid));
            at(5);
            assert_eq!(s.snd_open(22_050, 2), Ok(()));
            at(12);
            assert_eq!(s.snd_room(), Ok(77));
            assert_eq!(s.snd_write(&[1, 2, 3, 4]), Ok(2));
            at(40);
            assert_eq!(s.now_us(), 40_000);
            assert_eq!(s.snd_played(), Ok(1 << 40));
            at(1_000_000);
            assert_eq!(s.snd_close(), Err(Errno::BadHandle));
            s.end();
            let (_, _) = s.into_parts();
            assert_eq!(
                heard.0,
                [
                    ("open", 0),
                    ("open", 5),
                    ("room", 12),
                    ("write", 12),
                    ("pump", 40),
                    ("played", 40),
                    ("close", 1_000_000),
                    ("shut", 0),
                ]
            );
        });
    }

    #[test]
    fn every_file_error_has_the_c_library_s_number() {
        assert_eq!(errno(Error::NotFound), Errno::NoEntry);
        assert_eq!(errno(Error::BadPath), Errno::Invalid);
        assert_eq!(errno(Error::BadHandle), Errno::BadHandle);
        assert_eq!(errno(Error::NoHandle), Errno::NoHandle);
        assert_eq!(errno(Error::NoRoom), Errno::NoSpace);
        assert_eq!(errno(Error::Card(FatError::NoRoom)), Errno::NoSpace);
        assert_eq!(errno(Error::Card(FatError::Corrupt)), Errno::Io);
    }
}
