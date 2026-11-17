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
//! is what `fopen(.., "wb")` asks for; a file appended to or changed in
//! place is taken into the same room first and goes back whole (one
//! written file at a time, no bigger than the room: a game's log and its
//! settings, not its maps).

use core::fmt::Write as _;

use rp_fat::Error as FatError;
use rp_hal_api::BlockDevice;
use rp_hal_cabi::{Break, Errno, Event, FileStat, Gfx, Module, Sys, flags};

use crate::clock::Micros;
use crate::files::{Error, Fd, Files, Whence, unix_seconds};
use crate::gfx::Screen;
use crate::input::{Events, Turn};
use crate::module::{Modules, Refusal};
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
    /// The game's input, where the image has it.
    input: Option<&'a mut dyn Events>,
    /// The 2700G, where the image gives it to the program.
    screen: Option<&'a mut dyn Screen>,
    /// The program's modules, where the image is a runner (ADR-0039).
    library: Option<Library<'a>>,
    /// Why the last module was refused.
    refused: Option<Refusal>,
    /// The current folder as text, for `getcwd`: its bytes and how many
    /// (none: the root).
    cwd: ([u8; CWD], usize),
    /// Who hears what each path looked up cost, where the image counts.
    looks: Option<Looks>,
    /// The time the program's sound calls spent in the OS's mixer.
    meter: SoundMeter,
}

/// The time spent in the OS's mixer (`snd_write`, `snd_played`), told
/// once a minute with the mixer's own counts. Run 47's samples put 4.1%
/// of Half-Life's play on the instruction after `snd_write` turned the
/// interrupts back on (the mixer runs with them off, so the sampler could
/// not see inside), and the mixer's report came only at the run's end,
/// past the 64 KiB log: this says, in the run's log, what the mixer costs
/// and whether it mixes once or re-mixes (the iWMMXt question, quality
/// audit E1), before any change to it.
#[derive(Default)]
struct SoundMeter {
    us: u64,
    calls: u32,
    since_us: Option<u64>,
}

/// How an image hears what looking a path up cost (a game asks for
/// hundreds of files that are not there, and on a card each answer is
/// folder reads: Half-Life's start, R276): `reads` says the card's read
/// commands so far, and `looked` is told each path opened for reading,
/// asked about or listed, whether it was there, and the read commands
/// its answer took.
#[derive(Clone, Copy)]
pub struct Looks {
    /// The card's read commands so far.
    pub reads: fn() -> u32,
    /// A path was looked up: found or not, and the read commands it took.
    pub looked: fn(path: &[u8], found: bool, reads: u32),
    /// A file was opened for reading: its C handle and its path.
    pub opened: fn(fd: i32, path: &[u8]),
    /// A file was read: its C handle, where in it, the bytes asked and
    /// given, and the read commands that took (R281: a start's card time
    /// after its look-ups is its reads).
    pub read: fn(fd: i32, at: u64, asked: usize, got: usize, reads: u32),
    /// A folder's next entry was asked for: its C handle, whether there
    /// was one, and the read commands that took.
    pub listed: fn(fd: i32, more: bool, reads: u32),
}

/// The longest current folder's path.
const CWD: usize = 96;

/// What loads a big program's modules (ADR-0039).
pub struct Library<'a> {
    /// The loader, over the memory the modules were placed in.
    pub modules: Modules<'a>,
    /// The folder the module files are in (the runner's own: part of the
    /// OS, not of the game's data).
    pub folder: &'a [u8],
    /// Makes loaded bytes fit to run: the address and the length of what
    /// was just written (the CPU's caches hold the bytes that were there
    /// before; `rp_hal_pxa27x::cpu::clean_invalidate_range`).
    pub fit: fn(u32, u32),
}

/// The longest path of a module file (its folder, a slash, its name).
const MODULE_PATH: usize = 96;

/// A console as a `core::fmt` writer, for a line with numbers in it.
struct Line<'c, C: Console>(&'c mut C);

impl<C: Console> core::fmt::Write for Line<'_, C> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.put(s.as_bytes());
        Ok(())
    }
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
            screen: None,
            input: None,
            library: None,
            refused: None,
            cwd: ([0; CWD], 0),
            looks: None,
            meter: SoundMeter::default(),
        }
    }

    /// Counts a sound call that started at `t0` (microseconds) and, once
    /// a minute, says what they took and the mixer's counts.
    fn metered(&mut self, t0: u64) {
        let t1 = self.clock.at((self.ticks)());
        let m = &mut self.meter;
        m.us += t1.saturating_sub(t0);
        m.calls += 1;
        let since = *m.since_us.get_or_insert(t0);
        if t1.saturating_sub(since) >= 60_000_000 {
            let span = t1 - since;
            let _ = writeln!(
                Line(&mut self.console),
                "sound (the last {} s): the OS's mixer took {} ms in {} calls, {}.{}% of the time",
                span / 1_000_000,
                m.us / 1_000,
                m.calls,
                m.us * 100 / span.max(1),
                m.us * 1000 / span.max(1) % 10
            );
            if let Some(v) = self.sound.as_mut() {
                v.report(&mut Line(&mut self.console));
            }
            self.meter = SoundMeter {
                since_us: Some(t1),
                ..SoundMeter::default()
            };
        }
    }

    /// Has `looks` told of every path looked up from here on.
    pub fn set_looks(&mut self, looks: Looks) {
        self.looks = Some(looks);
    }

    /// `look` run, and its cost told to who counts ([`Looks`]): only a
    /// path that was found or was not there counts (a bad handle or a
    /// card error is not an answer about the path).
    fn looked<T>(
        &mut self,
        path: &[u8],
        look: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let Some(l) = self.looks else {
            return look(self);
        };
        let before = (l.reads)();
        let got = look(self);
        if matches!(got, Ok(_) | Err(Error::NotFound)) {
            (l.looked)(path, got.is_ok(), (l.reads)().wrapping_sub(before));
        }
        got
    }

    /// Makes the system a runner's: `rp_module_load` and its kin reach
    /// `library`.
    pub fn set_modules(&mut self, library: Library<'a>) {
        self.library = Some(library);
    }

    /// The loader, for what a runner asks before any C code runs (where
    /// the set of modules ends).
    pub fn modules(&mut self) -> Option<&mut Modules<'a>> {
        self.library.as_mut().map(|l| &mut l.modules)
    }

    /// Gives the system its input (the buttons and the touch chip): what
    /// `rp_input_open` and `rp_input_poll` reach.
    pub fn set_input(&mut self, input: &'a mut dyn Events) {
        self.input = Some(input);
    }

    /// Gives the system the 2700G: what `rp_gfx_open` and the calls after
    /// it reach.
    pub fn set_screen(&mut self, screen: &'a mut dyn Screen) {
        self.screen = Some(screen);
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
            _ => {
                let f = file(fd)?;
                let Some(l) = self.looks else {
                    return self.files.read(f, into).map_err(errno);
                };
                let at = self.files.seek(f, 0, Whence::Here).unwrap_or(0);
                let before = (l.reads)();
                let got = self.files.read(f, into).map_err(errno)?;
                (l.read)(fd, at, into.len(), got, (l.reads)().wrapping_sub(before));
                Ok(got)
            }
        }
    }

    fn open(&mut self, path: &[u8], how: i32) -> Result<i32, Errno> {
        let fd = match how & flags::ACCESS {
            0 if how & (flags::CREATE | flags::TRUNCATE | flags::APPEND) == 0 => {
                self.looked(path, |s| s.files.open(path))
            }
            // Reading only, with a flag that would change the file: no.
            0 => return Err(Errno::Invalid),
            // "w", "w+": made new, whatever was there replaced.
            _ if how & flags::TRUNCATE != 0 && how & flags::APPEND == 0 => self.files.create(path),
            // "a", "a+": every write after what is there. "r+": changed
            // from its start. Made if it may be.
            _ => self
                .files
                .reopen(path, how & flags::APPEND != 0, how & flags::CREATE != 0),
        }
        .map_err(errno)?;
        let fd = i32::from(fd) + FIRST;
        if let (Some(l), 0) = (self.looks, how & flags::ACCESS) {
            (l.opened)(fd, path);
        }
        Ok(fd)
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
        let before = self.heap.used();
        let got = self.heap.grow(by);
        // The heap's size is said on the console as it passes each MiB,
        // and when a request is refused: a run's log is where a program's
        // memory is read from (a game that runs out says only that it did).
        let now = (self.clock.at((self.ticks)()) / 1_000) as u32;
        match got {
            Some(_) if self.heap.used() >> 20 != before >> 20 => {
                let _ = writeln!(
                    Line(&mut self.console),
                    "heap: {} MiB in use at {now} ms",
                    self.heap.used() >> 20
                );
            }
            None => {
                let _ = writeln!(
                    Line(&mut self.console),
                    "heap: a request for {by} bytes refused with {} KiB in use at {now} ms",
                    before / 1024
                );
            }
            Some(_) => {}
        }
        got
    }

    fn stat(&mut self, path: &[u8]) -> Result<FileStat, Errno> {
        let s = self.looked(path, |s| s.files.stat(path)).map_err(errno)?;
        Ok(FileStat {
            folder: s.folder,
            size: s.size,
            modified: s.modified.as_ref().map_or(0, unix_seconds),
        })
    }

    fn dir_open(&mut self, path: &[u8]) -> Result<i32, Errno> {
        let fd = self
            .looked(path, |s| s.files.open_dir(path))
            .map_err(errno)?;
        Ok(i32::from(fd) + FIRST)
    }

    fn dir_next(&mut self, fd: i32, name: &mut [u8]) -> Result<Option<(usize, bool)>, Errno> {
        let f = file(fd)?;
        let before = self.looks.map(|l| (l.reads)());
        let e = self.files.next(f, name).map_err(errno)?;
        if let (Some(l), Some(before)) = (self.looks, before) {
            (l.listed)(fd, e.is_some(), (l.reads)().wrapping_sub(before));
        }
        Ok(e.map(|e| (e.name_len, e.folder)))
    }

    fn mkdir(&mut self, path: &[u8]) -> Result<(), Errno> {
        self.files.mkdir(path).map_err(errno)
    }

    fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Errno> {
        self.files.rename(from, to).map_err(errno)
    }

    fn chdir(&mut self, path: &[u8]) -> Result<(), Errno> {
        // The text first (it must fit), then the folder itself.
        let (mut text, mut len) = self.cwd;
        if matches!(path.first(), Some(b'/' | b'\\')) {
            len = 0;
        }
        for name in path.split(|c| *c == b'/' || *c == b'\\') {
            if name.is_empty() || name == b"." {
                continue;
            }
            let end = len + 1 + name.len();
            if end > CWD {
                return Err(Errno::Invalid);
            }
            text[len] = b'/';
            text[len + 1..end].copy_from_slice(name);
            len = end;
        }
        self.files.set_current(path).map_err(errno)?;
        self.cwd = (text, len);
        Ok(())
    }

    fn cwd(&mut self, into: &mut [u8]) -> Option<usize> {
        let path: &[u8] = if self.cwd.1 == 0 {
            b"/"
        } else {
            &self.cwd.0[..self.cwd.1]
        };
        into.get_mut(..path.len())?.copy_from_slice(path);
        Some(path.len())
    }

    fn dup(&mut self, fd: i32) -> Result<i32, Errno> {
        let new = self.files.dup(file(fd)?).map_err(errno)?;
        Ok(i32::from(new) + FIRST)
    }

    fn sync(&mut self, fd: i32) -> Result<(), Errno> {
        // A file read has nothing to put on the card; a file written goes
        // there whole when it is closed, so "it is there now" would be a
        // lie: said, not pretended.
        match self.files.size(file(fd)?) {
            Ok(_) if self.files.writing(file(fd)?) => Err(Errno::Invalid),
            Ok(_) => Ok(()),
            Err(e) => Err(errno(e)),
        }
    }

    fn now_us(&mut self) -> u64 {
        let us = self.clock.at((self.ticks)());
        // A game reads the clock every frame: the mixer's wake, if due.
        if let Some(v) = self.sound.as_mut() {
            v.pump((us / 1_000) as u32);
        }
        // And the pad's reading, so a press between two polls is kept.
        if let Some(i) = self.input.as_mut() {
            i.pump(us);
        }
        us
    }

    fn input_open(&mut self, turn: u8) -> Result<(), Errno> {
        let now = self.clock.at((self.ticks)());
        let input = self.input.as_mut().ok_or(Errno::NoSys)?;
        input.open(Turn::from_c(turn).ok_or(Errno::Invalid)?, now)
    }

    fn input_poll(&mut self) -> Result<Option<Event>, Errno> {
        let now = self.clock.at((self.ticks)());
        self.input.as_mut().ok_or(Errno::NoSys)?.poll(now)
    }

    fn gfx_open(&mut self) -> Result<Gfx, Errno> {
        self.screen.as_mut().ok_or(Errno::NoSys)?.open()
    }

    fn gfx_frame(&mut self, words: &[u32]) -> Result<(), Errno> {
        self.screen.as_mut().ok_or(Errno::NoSys)?.frame(words)
    }

    fn gfx_wait(&mut self) -> Result<(), Errno> {
        self.screen.as_mut().ok_or(Errno::NoSys)?.wait()
    }

    fn gfx_close(&mut self) -> Result<(), Errno> {
        self.screen.as_mut().ok_or(Errno::NoSys)?.close()
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
        let t0 = self.clock.at((self.ticks)());
        let r = {
            let (v, now) = self.voice()?;
            v.write(samples, now)
        };
        self.metered(t0);
        r
    }

    fn snd_played(&mut self) -> Result<u64, Errno> {
        let t0 = self.clock.at((self.ticks)());
        let r = {
            let (v, now) = self.voice()?;
            v.played(now)
        };
        self.metered(t0);
        r
    }

    fn snd_close(&mut self) -> Result<(), Errno> {
        let (v, now) = self.voice()?;
        v.close(now)
    }

    fn module_load(&mut self, name: &[u8]) -> Result<Module, Errno> {
        let Self {
            files,
            console,
            library,
            refused,
            ticks,
            clock,
            ..
        } = self;
        let Some(lib) = library.as_mut() else {
            return Err(Errno::NoSys);
        };
        let started = clock.at(ticks());
        let mut path = [0u8; MODULE_PATH];
        // From the card's root, wherever the program's current folder is.
        let len = lib.folder.len() + 2 + name.len();
        let loaded =
            if name.is_empty() || name.iter().any(|b| b"/\\:".contains(b)) || len > path.len() {
                // A module is a file of the runner's own folder, by its name.
                Err(Refusal::Unread)
            } else {
                path[0] = b'/';
                path[1..=lib.folder.len()].copy_from_slice(lib.folder);
                path[lib.folder.len() + 1] = b'/';
                path[lib.folder.len() + 2..len].copy_from_slice(name);
                match files.open(&path[..len]) {
                    Err(_) => Err(Refusal::Unread),
                    Ok(fd) => {
                        let got = lib.modules.load(&mut |into| files.read(fd, into).ok());
                        let _ = files.close(fd);
                        got
                    }
                }
            };
        match loaded {
            Ok(h) => {
                (lib.fit)(h.base, h.file_len);
                *refused = None;
                // Every load says what it took: a run's log is where its
                // start-up time is read from.
                console.put(b"module ");
                console.put(name);
                let _ = writeln!(
                    Line(console),
                    ": {} KiB at 0x{:08X} and {} KiB zeroed, read and checked in {} ms",
                    h.file_len.div_ceil(1024),
                    h.base,
                    (h.mem_len - h.file_len).div_ceil(1024),
                    (clock.at(ticks()) - started) / 1_000
                );
                Ok(Module {
                    handle: h.base as usize,
                    entry: h.entry as usize,
                    constructors: (h.init.0 as usize, h.init.1 as usize),
                })
            }
            Err(r) => {
                *refused = Some(r);
                console.put(b"module ");
                console.put(name);
                console.put(b" not loaded: ");
                console.put(r.reason().to_bytes());
                console.put(b"\n");
                Err(match r {
                    Refusal::Unread => Errno::NoEntry,
                    Refusal::Full => Errno::NoMemory,
                    _ => Errno::Invalid,
                })
            }
        }
    }

    fn module_free(&mut self, handle: usize) -> Result<(), Errno> {
        let lib = self.library.as_mut().ok_or(Errno::NoSys)?;
        let handle = u32::try_from(handle).map_err(|_| Errno::BadHandle)?;
        if lib.modules.free(handle) {
            Ok(())
        } else {
            Err(Errno::BadHandle)
        }
    }

    fn module_find(&mut self, handle: usize, name: &[u8]) -> Result<usize, Errno> {
        let lib = self.library.as_ref().ok_or(Errno::NoSys)?;
        let names = u32::try_from(handle)
            .ok()
            .and_then(|h| lib.modules.names(h))
            .ok_or(Errno::BadHandle)?;
        names.find(name).map(|at| at as usize).ok_or(Errno::NoEntry)
    }

    fn module_name(&mut self, handle: usize, function: usize) -> Result<usize, Errno> {
        let lib = self.library.as_ref().ok_or(Errno::NoSys)?;
        let names = u32::try_from(handle)
            .ok()
            .and_then(|h| lib.modules.names(h))
            .ok_or(Errno::BadHandle)?;
        u32::try_from(function)
            .ok()
            .and_then(|f| names.name_of(f))
            .map(|(at, _)| at as usize)
            .ok_or(Errno::NoEntry)
    }

    fn module_reason(&mut self) -> &'static core::ffi::CStr {
        match self.refused {
            Some(r) => r.reason(),
            None if self.library.is_some() => c"no module was refused",
            None => c"this system loads no modules",
        }
    }

    fn end(&mut self) {
        if let Some(v) = self.sound.as_mut() {
            v.report(&mut Line(&mut self.console));
            v.shut();
        }
        // The screen back to the image, if the program left it open.
        if let Some(g) = self.screen.as_mut() {
            let _ = g.close();
        }
        // What the folders' names took, for the run's log: whether the
        // room given was enough is read here, not guessed.
        if let Some(u) = self.files.remembered() {
            let _ = writeln!(
                Line(&mut self.console),
                "files: the names of {} folders kept ({} names, {} KiB); {} did not fit and were read from the card",
                u.folders,
                u.names,
                u.bytes.div_ceil(1024),
                u.refused
            );
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
            // The refused request is said too (the heap's own line).
            assert_eq!(
                said.0,
                b"out\nerr\nheap: a request for 1 bytes refused with 0 KiB in use at 0 ms\n"
            );
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
            // Reading only with a flag that would change the file: refused.
            assert_eq!(s.open(b"a.txt", R | flags::TRUNCATE), Err(Errno::Invalid));
            // "a": after what is there. "r+": from its start, the rest
            // kept. "w+": new, and what is written reads back.
            let a = s.open(b"a.txt", A).unwrap();
            assert_eq!(s.write(a, b"AB"), Ok(2));
            assert_eq!(s.close(a), Ok(()));
            let rw = s.open(b"a.txt", R_PLUS).unwrap();
            assert_eq!(s.write(rw, b"xy"), Ok(2));
            // Not on the card until it is closed: fsync says so.
            assert_eq!(s.sync(rw), Err(Errno::Invalid));
            assert_eq!(s.close(rw), Ok(()));
            let r = s.open(b"a.txt", R).unwrap();
            let mut got = [0u8; 16];
            assert_eq!(s.read(r, &mut got), Ok(12));
            assert_eq!(&got[..12], b"xy23456789AB", "{kind}");
            assert_eq!(s.sync(r), Ok(()), "nothing to put on the card");
            assert_eq!(s.sync(99), Err(Errno::BadHandle));
            assert_eq!(s.close(r), Ok(()));
            assert_eq!(s.open(b"none.txt", R_PLUS), Err(Errno::NoEntry));
            // The room is 16 bytes: a file that does not fit is not taken
            // in to be appended to.
            let w = s.open(b"b.txt", W_PLUS).unwrap();
            assert_eq!(s.write(w, b"0123456789abcdef"), Ok(16));
            assert_eq!(s.seek(w, 0, 0), Ok(0));
            assert_eq!(s.read(w, &mut got), Ok(16));
            assert_eq!(s.close(w), Ok(()));
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

    #[test]
    fn input_calls_reach_the_pad_with_the_clock_s_time_and_are_refused_without_one() {
        use crate::input::{Input, Pad};
        use rp_drv_pxa_keypad::{Button, Pressed};
        use rp_hal_cabi::event;
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        static UP: AtomicBool = AtomicBool::new(false);
        /// Wifi held all along; Up as the test says.
        struct Buttons;
        impl Pad for Buttons {
            fn pressed(&mut self) -> Pressed {
                let p = Pressed::NONE.with(Button::Wifi);
                if UP.load(Ordering::Relaxed) {
                    p.with(Button::Up)
                } else {
                    p
                }
            }
            fn pen(&mut self) -> Option<rp_touch_cal::Sample> {
                None
            }
        }
        each(|vol, kind| {
            COUNTER.store(0, Ordering::Relaxed);
            UP.store(false, Ordering::Relaxed);
            let at = |ms: u32| COUNTER.store(ms * 3_250, Ordering::Relaxed);
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 16][..];
            let mut said = Said(Vec::new());
            let mut pad = Input::new(Buttons, rp_touch_cal::Calibration::X51V_DEFAULT);
            let files: Files<'_, Disk, 2> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0, 0), || {
                COUNTER.load(Ordering::Relaxed)
            });
            // Without a pad: not something this system does.
            assert_eq!(s.input_open(0), Err(Errno::NoSys), "{kind}");
            assert_eq!(s.input_poll(), Err(Errno::NoSys));
            s.set_input(&mut pad);
            assert_eq!(s.input_poll(), Err(Errno::BadHandle), "not open yet");
            assert_eq!(s.input_open(3), Err(Errno::Invalid), "no such turn");
            assert_eq!(s.input_open(u8::MAX), Err(Errno::Invalid));
            at(5);
            assert_eq!(s.input_open(event::TURN_RIGHT), Ok(()));
            assert_eq!(s.input_open(event::TURN_RIGHT), Err(Errno::Busy));
            assert_eq!(s.input_poll(), Ok(None), "held before the open");
            // Up pressed and let go again between two polls: the clock's
            // read at 20 ms sees the press, the poll at 30 ms the release.
            UP.store(true, Ordering::Relaxed);
            at(20);
            assert_eq!(s.now_us(), 20_000);
            UP.store(false, Ordering::Relaxed);
            at(30);
            assert_eq!(
                s.input_poll(),
                Ok(Some(Event::key(event::KEY_RIGHT, true, 20_000)))
            );
            assert_eq!(
                s.input_poll(),
                Ok(Some(Event::key(event::KEY_RIGHT, false, 30_000)))
            );
            assert_eq!(s.input_poll(), Ok(None));
        });
    }

    #[test]
    fn folders_are_listed_made_and_named_and_a_path_says_what_it_is() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 64][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 3> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0, 0), || 0);
            assert_eq!(s.mkdir(b"valve"), Ok(()));
            assert_eq!(s.mkdir(b"valve"), Ok(()), "twice is no error");
            assert_eq!(s.mkdir(b"valve/save"), Ok(()));
            assert_eq!(s.mkdir(b"none/save"), Err(Errno::NoEntry), "{kind}");
            let w = s.open(b"valve/config.cfg", W).unwrap();
            assert_eq!(s.write(w, b"cl_showfps 1\n"), Ok(13));
            assert_eq!(s.close(w), Ok(()));
            // What a path is.
            let st = s.stat(b"valve/config.cfg").unwrap();
            assert_eq!((st.folder, st.size), (false, 13));
            assert!(s.stat(b"valve/save").unwrap().folder);
            assert!(s.stat(b"/").unwrap().folder);
            assert_eq!(s.stat(b"valve/none"), Err(Errno::NoEntry));
            // The folder listed: its two entries, then its end.
            let d = s.dir_open(b"valve").unwrap();
            let mut names = Vec::new();
            let mut name = [0u8; 255];
            while let Some((n, folder)) = s.dir_next(d, &mut name).unwrap() {
                names.push((String::from_utf8_lossy(&name[..n]).to_lowercase(), folder));
            }
            names.sort();
            assert_eq!(
                names,
                [
                    ("config.cfg".to_string(), false),
                    ("save".to_string(), true)
                ],
                "{kind}"
            );
            // A second handle on a file; a folder's handle is not a file's.
            let r = s.open(b"valve/config.cfg", R).unwrap();
            let r2 = s.dup(r).unwrap();
            assert_ne!(r, r2);
            assert_eq!(s.dup(r), Err(Errno::NoHandle), "three handles");
            assert_eq!(s.read(d, &mut name), Err(Errno::BadHandle));
            assert_eq!(s.dir_next(r, &mut name), Err(Errno::BadHandle));
            for fd in [d, r, r2] {
                assert_eq!(s.close(fd), Ok(()));
            }
            assert_eq!(s.dir_open(b"valve/config.cfg"), Err(Errno::Invalid));
            assert_eq!(s.dir_open(b"none"), Err(Errno::NoEntry));
            assert_eq!(s.dup(1), Err(Errno::BadHandle), "the console");
            // Renamed; the old name is gone.
            assert_eq!(s.rename(b"valve/config.cfg", b"valve/save/old.cfg"), Ok(()));
            assert_eq!(s.stat(b"valve/config.cfg"), Err(Errno::NoEntry));
            assert_eq!(s.stat(b"valve/save/old.cfg").map(|st| st.size), Ok(13));
            assert_eq!(s.rename(b"valve/none", b"valve/x"), Err(Errno::NoEntry));
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
    fn a_runner_loads_its_modules_by_name_from_its_own_folder() {
        use crate::module::testing::module;
        const BASE: u32 = 0xA900_0000;
        const RUNNER: u64 = 77;
        static FITTED: std::sync::Mutex<Vec<(u32, u32)>> = std::sync::Mutex::new(Vec::new());
        each(|vol, kind| {
            FITTED.lock().unwrap().clear();
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 100_000][..];
            let pool = &mut vec![0xEEu8; 64 * 1024][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 4> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0x1000, 0x3000), || 0);
            // Without a loader: not something this system does.
            assert_eq!(s.module_load(b"server.rpm"), Err(Errno::NoSys));
            assert_eq!(s.module_find(BASE as usize, b"x"), Err(Errno::NoSys));
            assert!(s.module_reason().to_bytes().starts_with(b"this system"));
            // The files, as the build writes them.
            let good = module(
                RUNNER,
                BASE + 0x1000,
                5000,
                &[
                    ("GetEntityAPI", BASE + 0x1050),
                    ("worldspawn", BASE + 0x1080),
                ],
                &[BASE + 0x1060],
                512,
            );
            let other = module(RUNNER + 1, BASE + 0x4000, 100, &[], &[], 0);
            let mut cut = good.clone();
            cut.truncate(good.len() - 100);
            s.files().mkdir(b"rp-game").unwrap();
            for (name, bytes) in [
                (&b"rp-game/server.rpm"[..], &good),
                (b"rp-game/other.rpm", &other),
                (b"rp-game/cut.rpm", &cut),
            ] {
                let w = s.open(name, W).unwrap();
                assert_eq!(s.write(w, bytes), Ok(bytes.len()), "{kind}");
                s.close(w).unwrap();
            }
            s.set_modules(Library {
                modules: Modules::new(pool, BASE, RUNNER),
                folder: b"rp-game",
                fit: |at, len| FITTED.lock().unwrap().push((at, len)),
            });
            assert!(s.modules().is_some());
            let m = s.module_load(b"server.rpm").expect("loaded");
            assert_eq!(m.handle, (BASE + 0x1000) as usize);
            assert_eq!(m.entry, (BASE + 0x1000) as usize + 72);
            assert_eq!(m.constructors.1 - m.constructors.0, 4);
            // What was written was made fit to run: the file's bytes.
            assert_eq!(
                *FITTED.lock().unwrap(),
                [(BASE + 0x1000, good.len() as u32)],
                "{kind}"
            );
            assert_eq!(
                s.module_find(m.handle, b"worldspawn"),
                Ok((BASE + 0x1080) as usize)
            );
            assert_eq!(s.module_find(m.handle, b"nothing"), Err(Errno::NoEntry));
            assert_eq!(
                s.module_find(m.handle + 4, b"worldspawn"),
                Err(Errno::BadHandle)
            );
            let name = s
                .module_name(m.handle, (BASE + 0x1050) as usize)
                .expect("named");
            assert!(name > m.handle && name < m.handle + good.len());
            assert_eq!(
                s.module_name(m.handle, (BASE + 0x1054) as usize),
                Err(Errno::NoEntry)
            );
            // Refused, each with its reason said on the console too.
            assert_eq!(s.module_load(b"other.rpm"), Err(Errno::Invalid));
            assert!(
                s.module_reason()
                    .to_bytes()
                    .starts_with(b"it was built for another")
            );
            assert_eq!(s.module_load(b"cut.rpm"), Err(Errno::Invalid));
            assert!(s.module_reason().to_bytes().starts_with(b"it is damaged"));
            assert_eq!(s.module_load(b"absent.rpm"), Err(Errno::NoEntry));
            // A name is a name: no path out of the runner's folder.
            for name in [
                &b"../rp-game/server.rpm"[..],
                b"sub/server.rpm",
                b"a:server.rpm",
                b"",
            ] {
                assert_eq!(s.module_load(name), Err(Errno::NoEntry), "{name:?}");
            }
            // The handles a load used are given back: this system has 4.
            for _ in 0..6 {
                assert!(s.module_load(b"server.rpm").is_ok());
            }
            // The program's current folder does not move the runner's.
            s.files().mkdir(b"Ports").unwrap();
            assert_eq!(s.chdir(b"/Ports"), Ok(()));
            assert!(s.module_load(b"server.rpm").is_ok(), "{kind}");
            assert_eq!(s.chdir(b"/"), Ok(()));
            assert!(
                s.module_reason()
                    .to_bytes()
                    .starts_with(b"no module was refused")
            );
            assert_eq!(s.module_free(m.handle), Ok(()));
            assert_eq!(s.module_free(m.handle), Err(Errno::BadHandle));
            assert_eq!(
                s.module_find(m.handle, b"worldspawn"),
                Err(Errno::BadHandle)
            );
            let _ = s;
            let text = String::from_utf8_lossy(&said.0).into_owned();
            assert!(
                text.contains(
                    "module other.rpm not loaded: it was built for another build of the runner"
                ),
                "{text}"
            );
            assert!(text.contains("module cut.rpm not loaded: it is damaged"));
            assert!(text.contains("module absent.rpm not loaded: its file is not there"));
            // A load says its size, its place and its time.
            assert!(
                text.contains("module server.rpm: 6 KiB at 0xA9001000 and 1 KiB zeroed, read and checked in 0 ms"),
                "{text}"
            );
        });
    }

    #[test]
    fn the_heap_says_each_mib_it_passes_and_a_request_it_refuses() {
        each(|vol, _| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 1000][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 4> = Files::new(vol, scratch, room);
            let heap = Break::new(0x10_0000, 0x10_0000 + (3 << 20));
            // 3,250 ticks a millisecond.
            let mut s = System::new(files, &mut said, heap, || 6_500);
            // Small steps say nothing until a MiB is passed.
            assert!(s.grow(1000).is_some());
            assert!(s.grow((1 << 20) - 1000 - 1).is_some());
            assert!(s.grow(1).is_some());
            assert!(s.grow(1 << 20).is_some());
            // Given back and taken again: said again (the log shows both).
            assert!(s.grow(-(1 << 20)).is_some());
            assert!(s.grow(2 << 20).is_some());
            // More than is left: refused, nothing moved.
            assert_eq!(s.grow(1), None);
            assert_eq!(s.heap_used(), 3 << 20);
            let _ = s;
            let text = String::from_utf8_lossy(&said.0).into_owned();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(
                lines,
                [
                    "heap: 1 MiB in use at 0 ms",
                    "heap: 2 MiB in use at 0 ms",
                    "heap: 1 MiB in use at 0 ms",
                    "heap: 3 MiB in use at 0 ms",
                    "heap: a request for 1 bytes refused with 3072 KiB in use at 0 ms",
                ]
            );
        });
    }

    #[test]
    fn a_program_s_current_folder_is_where_its_relative_paths_start() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 100_000][..];
            let mut said = Said(Vec::new());
            let files: Files<'_, Disk, 4> = Files::new(vol, scratch, room);
            let mut s = System::new(files, &mut said, Break::new(0x1000, 0x3000), || 0);
            let put = |s: &mut System<'_, Disk, 4, &mut Said>, path: &[u8], text: &[u8]| {
                let w = s.open(path, W).unwrap();
                s.write(w, text).unwrap();
                s.close(w).unwrap();
            };
            let cwd = |s: &mut System<'_, Disk, 4, &mut Said>| {
                let mut buf = [0u8; 64];
                let n = s.cwd(&mut buf).unwrap();
                buf[..n].to_vec()
            };
            s.mkdir(b"Ports").unwrap();
            s.mkdir(b"Ports/valve").unwrap();
            put(&mut s, b"Ports/valve/liblist.gam", b"game");
            put(&mut s, b"top.txt", b"top");
            assert_eq!(cwd(&mut s), b"/", "{kind}");
            // From the root, the game's file needs its whole path.
            assert_eq!(s.open(b"valve/liblist.gam", R), Err(Errno::NoEntry));
            assert_eq!(s.chdir(b"/Ports"), Ok(()));
            assert_eq!(cwd(&mut s), b"/Ports");
            // Now relative names start in Ports; a slash still means the root.
            let r = s.open(b"valve/liblist.gam", R).unwrap();
            s.close(r).unwrap();
            let r = s.open(b"./valve\\liblist.gam", R).unwrap();
            s.close(r).unwrap();
            assert!(s.stat(b"valve").unwrap().folder);
            assert_eq!(s.open(b"top.txt", R), Err(Errno::NoEntry));
            let r = s.open(b"/top.txt", R).unwrap();
            s.close(r).unwrap();
            // The folder itself lists, by "." and by nothing.
            for here in [&b"."[..], b""] {
                let d = s.dir_open(here).unwrap();
                let mut name = [0u8; 32];
                let (n, folder) = s.dir_next(d, &mut name).unwrap().unwrap();
                assert_eq!((&name[..n], folder), (&b"valve"[..], true), "{kind}");
                s.close(d).unwrap();
            }
            // A step further down, relative; a file made there lands there.
            assert_eq!(s.chdir(b"valve"), Ok(()));
            assert_eq!(cwd(&mut s), b"/Ports/valve");
            put(&mut s, b"config.cfg", b"cfg");
            assert_eq!(s.stat(b"/Ports/valve/config.cfg").unwrap().size, 3);
            // Not a folder, not there, up: refused, and nothing moved.
            assert_eq!(s.chdir(b"liblist.gam"), Err(Errno::Invalid));
            assert_eq!(s.chdir(b"nowhere"), Err(Errno::NoEntry));
            assert_eq!(s.chdir(b".."), Err(Errno::Invalid));
            assert_eq!(s.chdir(&[b'a'; 200]), Err(Errno::Invalid));
            assert_eq!(cwd(&mut s), b"/Ports/valve");
            // A buffer too small for the path gets none of it.
            assert_eq!(s.cwd(&mut [0u8; 5]), None);
            // Back to the root.
            assert_eq!(s.chdir(b"/"), Ok(()));
            assert_eq!(cwd(&mut s), b"/");
            let r = s.open(b"top.txt", R).unwrap();
            s.close(r).unwrap();
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
