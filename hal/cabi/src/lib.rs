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

/// What a path names, as `stat` fills it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    /// A folder.
    pub folder: bool,
    /// Its size in bytes.
    pub size: u64,
    /// When it was last changed, in seconds since 1970 (0: not known).
    pub modified: i64,
}

/// picolibc's `struct stat` (`sys/stat.h`), the fields a file system
/// without owners or links can fill: the kind, the size, the time.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct CStat {
    dev: u64,
    ino: u64,
    /// `S_IFDIR` or `S_IFREG`, with read and write for everyone.
    pub mode: u32,
    nlink: u64,
    uid: u32,
    gid: u32,
    rdev: u64,
    /// Bytes.
    pub size: i64,
    blksize: i32,
    blocks: i64,
    atime: (i64, i32),
    /// Seconds since 1970.
    pub mtime: (i64, i32),
    ctime: (i64, i32),
}

impl CStat {
    /// `S_IFDIR | 0777`.
    pub const FOLDER: u32 = 0o040_777;
    /// `S_IFREG | 0666`.
    pub const FILE: u32 = 0o100_666;

    /// The C structure for `s`.
    #[must_use]
    pub fn of(s: &FileStat) -> Self {
        let size = i64::try_from(s.size).unwrap_or(i64::MAX);
        Self {
            mode: if s.folder { Self::FOLDER } else { Self::FILE },
            nlink: 1,
            size,
            blksize: 512,
            blocks: size / 512 + i64::from(size % 512 != 0),
            atime: (s.modified, 0),
            mtime: (s.modified, 0),
            ctime: (s.modified, 0),
            ..Self::default()
        }
    }
}

/// The longest name in a `struct dirent` (picolibc's `NAME_MAX`).
pub const NAME_MAX: usize = 255;

/// picolibc's `struct dirent` (`sys/dirent.h`).
#[repr(C)]
pub struct Dirent {
    ino: u64,
    /// `DT_DIR` (3) or `DT_REG` (6).
    pub kind: u8,
    /// The name, NUL-terminated.
    pub name: [u8; NAME_MAX + 1],
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

/// The screen a C program draws on through the 2700G (`struct rp_gfx` in
/// `ports/include/retropane.h`): what `rp_gfx_open` fills in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Gfx {
    /// The picture drawn into, upright (480 x 640 on the X51v).
    pub width: u16,
    /// See [`Self::width`].
    pub height: u16,
    /// The program's texture memory as the 2700G addresses it (what a
    /// state block's texture words name).
    pub tex_at: u32,
    /// Its size in bytes.
    pub tex_bytes: u32,
    /// The same memory as the CPU writes it.
    pub tex: *mut core::ffi::c_void,
    /// The picture as drawn (`width` x `height` RGB565, rows of `width`
    /// x 2 bytes), for a test's eye: read only, after `rp_gfx_wait`.
    pub screen: *const core::ffi::c_void,
    /// [`gfx_flags`]: how the screen takes a frame's words.
    pub flags: u32,
}

/// [`Gfx::flags`] (`RP_GFX_*` in `retropane.h`).
pub mod gfx_flags {
    /// A frame's words are read after `rp_gfx_frame_parts` returns (the
    /// chip fed by DMA while the program goes on: ref-rp.md step 4f), up
    /// to the next `rp_gfx_frame_parts` (or `rp_gfx_wait`) returning: the
    /// program keeps them (two buffers, one written while the other is
    /// read).
    pub const KEEP: u32 = 1;
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

/// A module the system loaded (ADR-0039: a big program's library, placed
/// at build time), as the C boundary needs it to start it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Module {
    /// Its handle: the address it is loaded at.
    pub handle: usize,
    /// The address of its entry, `int (int, char **)`; 0 for a library.
    pub entry: usize,
    /// The addresses holding its constructors, one function's address a
    /// word, from `.0` up to `.1`. Whoever answers [`Sys::module_load`]
    /// has checked that each is code of the module it loaded.
    pub constructors: (usize, usize),
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
    /// A large block from the heap's top region (`rp_big_alloc`,
    /// [`Break::big_alloc`]): its address, or `None` (the caller uses
    /// `malloc`). None by default.
    fn big_alloc(&mut self, _size: usize) -> Option<usize> {
        None
    }
    /// Frees a block [`Self::big_alloc`] gave (`rp_big_free`); false if
    /// `at` is not one.
    fn big_free(&mut self, _at: usize) -> bool {
        false
    }
    /// The size of the large block at `at`, 0 if it is not one
    /// (`rp_big_size`).
    fn big_size(&mut self, _at: usize) -> usize {
        0
    }
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
    /// What `path` names (`stat`).
    ///
    /// # Errors
    /// Nothing of that name; no files here (`ENOSYS`, the default).
    fn stat(&mut self, _path: &[u8]) -> Result<FileStat, Errno> {
        Err(Errno::NoSys)
    }
    /// Opens the folder at `path` for listing (`opendir`): a handle that
    /// [`Self::close`] closes.
    ///
    /// # Errors
    /// No such folder, or no handle left.
    fn dir_open(&mut self, _path: &[u8]) -> Result<i32, Errno> {
        Err(Errno::NoSys)
    }
    /// The folder's next entry (`readdir`): its name into `name`, the
    /// bytes used and whether it is a folder; `None` at the end.
    ///
    /// # Errors
    /// Not a folder's handle.
    fn dir_next(&mut self, _fd: i32, _name: &mut [u8]) -> Result<Option<(usize, bool)>, Errno> {
        Err(Errno::NoSys)
    }
    /// Makes the folder at `path` (`mkdir`).
    ///
    /// # Errors
    /// No such parent, or a file of that name (`EEXIST` is not said for a
    /// folder already there: making it twice is no error).
    fn mkdir(&mut self, _path: &[u8]) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// Gives `from` the path `to` (`rename`); a file at `to` makes way.
    ///
    /// # Errors
    /// Nothing at `from`, or no folder for `to`.
    fn rename(&mut self, _from: &[u8], _to: &[u8]) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// A second handle on what `fd` has open (`dup`).
    ///
    /// # Errors
    /// Not an open file, or no handle left.
    fn dup(&mut self, _fd: i32) -> Result<i32, Errno> {
        Err(Errno::NoSys)
    }
    /// Makes the folder at `path` the one relative paths start from
    /// (`chdir`).
    ///
    /// # Errors
    /// No such folder; no files here (`ENOSYS`, the default).
    fn chdir(&mut self, _path: &[u8]) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// The current folder's path into `into` (`getcwd`): the bytes used,
    /// or `None` if it does not fit. The card's root, "/", by default.
    fn cwd(&mut self, into: &mut [u8]) -> Option<usize> {
        *into.first_mut()? = b'/';
        Some(1)
    }
    /// Asks that what was written to `fd` be on the card now (`fsync`).
    ///
    /// # Errors
    /// Not an open handle; or it cannot be promised (a written file goes
    /// onto the card whole when it is closed: `EINVAL`).
    fn sync(&mut self, _fd: i32) -> Result<(), Errno> {
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
    /// Takes the screen for the program's 3D frames (`rp_gfx_open`): the
    /// 2700G's tile accelerator set up, the program's texture memory.
    ///
    /// # Errors
    /// No 2700G here (`ENOSYS`, the default), open already (`EBUSY`), or
    /// the chip would not set up (`EIO`).
    fn gfx_open(&mut self) -> Result<Gfx, Errno> {
        Err(Errno::NoSys)
    }
    /// One frame's words (`rp_gfx_frame`, `rp_gfx_frame_parts`), in parts
    /// drawn in their order: checked, fed to the tile accelerator behind a
    /// background, drawn into the screen.
    ///
    /// # Errors
    /// Not open (`EBADF`), a word the check refuses (`EINVAL`: nothing of
    /// the frame drawn), or the chip did not finish (`EIO`).
    fn gfx_frame(&mut self, _parts: &[&[u32]]) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// Waits until the last frame is drawn (`rp_gfx_wait`): before the
    /// program writes over texture memory that frame may read.
    ///
    /// # Errors
    /// Not open (`EBADF`), or the render did not finish (`EIO`).
    fn gfx_wait(&mut self) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// Gives the screen back (`rp_gfx_close`).
    ///
    /// # Errors
    /// Not open (`EBADF`).
    fn gfx_close(&mut self) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// Loads the module file `name` from the program's own folder
    /// (`rp_module_load`): read to its address, checked, its zeroed RAM
    /// zeroed, fit to run. Its constructors are the caller's to run.
    ///
    /// # Errors
    /// No modules here (`ENOSYS`, the default); the file is not there; it
    /// was refused ([`Self::module_reason`] says why).
    fn module_load(&mut self, _name: &[u8]) -> Result<Module, Errno> {
        Err(Errno::NoSys)
    }
    /// Forgets the loaded module `handle` (`rp_module_free`).
    ///
    /// # Errors
    /// Not a loaded module.
    fn module_free(&mut self, _handle: usize) -> Result<(), Errno> {
        Err(Errno::NoSys)
    }
    /// The address of the function `name` that the loaded module `handle`
    /// exports (`rp_module_find`).
    ///
    /// # Errors
    /// Not a loaded module, or no such name.
    fn module_find(&mut self, _handle: usize, _name: &[u8]) -> Result<usize, Errno> {
        Err(Errno::NoSys)
    }
    /// The address of the name (a NUL-terminated string in the module's
    /// own memory) of the exported function that starts at `function`
    /// (`rp_module_name`).
    ///
    /// # Errors
    /// Not a loaded module, or no exported function starts there.
    fn module_name(&mut self, _handle: usize, _function: usize) -> Result<usize, Errno> {
        Err(Errno::NoSys)
    }
    /// Why the last module was refused, in plain words
    /// (`rp_module_error`).
    fn module_reason(&mut self) -> &'static core::ffi::CStr {
        c"this system loads no modules"
    }
    /// The program is over (it returned, or called `exit`): stop what it
    /// left running (its sound). Nothing by default.
    fn end(&mut self) {}
}

/// Large blocks the heap's top region keeps track of at once
/// ([`Break::big_alloc`]); past them a large request goes to `malloc`.
pub const BIG_BLOCKS: usize = 256;

/// The alignment and granule of a large block: a cache line.
const BIG_ALIGN: usize = 32;

/// One large block of the top region: where, how long, in use or free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Big {
    at: usize,
    size: usize,
    used: bool,
}

/// The C heap's room: one run of RAM whose used part grows from its start
/// (`sbrk`'s model; picolibc's `malloc` asks for more as it needs it), and,
/// from its end down, a region for large blocks.
///
/// R310 (Half-Life's third map change out of memory, 2026-11-17): the
/// engine's pools ask for map-sized blocks (2.7 MB of surfaces) and free
/// them at the next map; under one first-fit `malloc` the small blocks that
/// live on (the server's, the sound's) settle in those holes, so the C heap
/// held 28 MiB with 7 MiB free in 564 holes and no hole for the next map's
/// block. dlmalloc and glibc keep large requests apart (`mmap` past a
/// threshold, given back whole on `free`); the X51v's port layer has no
/// MMU-backed `mmap`, so the large blocks get their own region at the
/// heap's top, growing down to meet `sbrk`'s break: best fit over an
/// address-ordered table, neighbours joined on free, and a free run at the
/// region's bottom given back to the break. Small blocks can never split a
/// large one's hole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Break {
    start: usize,
    end: usize,
    at: usize,
    /// The large blocks' region is `low..end`.
    low: usize,
    big: [Big; BIG_BLOCKS],
    /// Entries of `big` in use, by address from `low` up.
    count: usize,
}

impl Break {
    /// A heap of the addresses `start..end`, none of it used.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        let end = if end < start { start } else { end };
        Self {
            start,
            end,
            at: start,
            low: end,
            big: [Big {
                at: 0,
                size: 0,
                used: false,
            }; BIG_BLOCKS],
            count: 0,
        }
    }

    /// Moves the end of the used part by `by` and returns where it was;
    /// `None`, and nothing moved, if that leaves the room at either end
    /// (the top end is where the large blocks' region starts).
    pub fn grow(&mut self, by: isize) -> Option<usize> {
        let old = self.at;
        let new = old.checked_add_signed(by)?;
        if new < self.start || new > self.low {
            return None;
        }
        self.at = new;
        Some(old)
    }

    /// Bytes in use: the break's part and the large blocks' region.
    #[must_use]
    pub const fn used(&self) -> usize {
        (self.at - self.start) + (self.end - self.low)
    }

    /// A large block of at least `size` bytes, 32-byte aligned, from the
    /// top region; `None` if there is no room or the table is full (the
    /// caller then asks `malloc`).
    pub fn big_alloc(&mut self, size: usize) -> Option<usize> {
        let size = size.checked_add(BIG_ALIGN - 1)? & !(BIG_ALIGN - 1);
        if size == 0 {
            return None;
        }
        // Best fit among the free blocks.
        let best = (0..self.count)
            .filter(|&i| !self.big[i].used && self.big[i].size >= size)
            .min_by_key(|&i| self.big[i].size);
        if let Some(i) = best {
            let b = self.big[i];
            if b.size == size {
                self.big[i].used = true;
                return Some(b.at);
            }
            // Split: the block at the free run's top, the rest stays free
            // below it (towards the region's bottom, which can go back).
            if self.count == BIG_BLOCKS {
                return None;
            }
            self.big.copy_within(i + 1..self.count, i + 2);
            self.count += 1;
            self.big[i].size = b.size - size;
            let at = b.at + b.size - size;
            self.big[i + 1] = Big {
                at,
                size,
                used: true,
            };
            return Some(at);
        }
        // A new block below the region, if the break leaves room.
        if self.count == BIG_BLOCKS {
            return None;
        }
        let at = self.low.checked_sub(size)? & !(BIG_ALIGN - 1);
        if at < self.at {
            return None;
        }
        let gap = self.low - at;
        self.big.copy_within(0..self.count, 1);
        self.count += 1;
        self.big[0] = Big {
            at,
            size: gap,
            used: true,
        };
        self.low = at;
        Some(at)
    }

    /// The size of the large block at `at` (0 if `at` starts none in use).
    #[must_use]
    pub fn big_size(&self, at: usize) -> usize {
        self.find(at).map_or(0, |i| self.big[i].size)
    }

    /// Frees the large block at `at`; false if `at` starts none in use.
    pub fn big_free(&mut self, at: usize) -> bool {
        let Some(mut i) = self.find(at) else {
            return false;
        };
        self.big[i].used = false;
        // Joined with a free neighbour above, then below.
        if i + 1 < self.count && !self.big[i + 1].used {
            self.big[i].size += self.big[i + 1].size;
            self.big.copy_within(i + 2..self.count, i + 1);
            self.count -= 1;
        }
        if i > 0 && !self.big[i - 1].used {
            self.big[i - 1].size += self.big[i].size;
            self.big.copy_within(i + 1..self.count, i);
            self.count -= 1;
            i -= 1;
        }
        // A free run at the region's bottom goes back to the break.
        if i == 0 {
            self.low += self.big[0].size;
            self.big.copy_within(1..self.count, 0);
            self.count -= 1;
        }
        true
    }

    /// Bytes free inside the large blocks' region (its holes).
    #[must_use]
    pub fn big_free_bytes(&self) -> usize {
        self.big[..self.count]
            .iter()
            .filter(|b| !b.used)
            .map(|b| b.size)
            .sum()
    }

    fn find(&self, at: usize) -> Option<usize> {
        let i = self.big[..self.count].binary_search_by_key(&at, |b| b.at).ok()?;
        self.big[i].used.then_some(i)
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
    use super::{CStat, Dirent, Errno, Event, Gfx, Module, NAME_MAX, Sys};
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
        /// The table of the addresses a runner exports to its modules,
        /// which the build links into it (xtask/src/module.rs), and its
        /// end. Only a runner has them, and only a runner asks.
        static rp_runner_table: [u8; 0];
        static rp_runner_table_end: [u8; 0];
    }

    /// The bytes of the table of addresses this image exports to its
    /// modules (ADR-0039 rule 3: their hash is the runner build's
    /// identity). Only in an image the build linked the table into.
    #[must_use]
    pub fn runner_table() -> &'static [u8] {
        let start = core::ptr::addr_of!(rp_runner_table).cast::<u8>();
        let end = core::ptr::addr_of!(rp_runner_table_end).cast::<u8>();
        // SAFETY: the two symbols bound one object of the image's
        // read-only data, made by the build with the second right after
        // the first's last word; it is never written.
        unsafe { core::slice::from_raw_parts(start, (end as usize).saturating_sub(start as usize)) }
    }

    /// Loads the module `name` and runs the constructors of its global
    /// objects.
    fn load_module(name: &[u8]) -> Result<Module, Errno> {
        // The lock is let go before any of the module's code runs: a
        // constructor may call the C library, which comes back here.
        let module = with(|s| s.module_load(name))?;
        let mut at = module.constructors.0;
        while at < module.constructors.1 {
            // SAFETY: the system that loaded the module checked that these
            // words lie in its loaded bytes and that each holds the
            // address of code in the same module, whose check sum held
            // (`rp_port::module`); the build put there the functions the
            // compiler made to construct its global objects, which take
            // and return nothing. The loaded bytes were made fit to run.
            unsafe {
                let constructor = (at as *const unsafe extern "C" fn()).read();
                constructor();
            }
            at += core::mem::size_of::<usize>();
        }
        Ok(module)
    }

    /// The most arguments [`run_module`] passes on.
    pub const ARGS: usize = 32;

    /// Loads the program module `name` and runs it with `args` as its
    /// command line (the first is the program's own name): its result,
    /// with what it printed pushed out.
    ///
    /// # Errors
    /// The module was not loaded ([`Sys::module_reason`]), is a library
    /// with no entry, or there are more than [`ARGS`] arguments.
    pub fn run_module(name: &[u8], args: &[&'static CStr]) -> Result<i32, Errno> {
        if args.len() > ARGS {
            return Err(Errno::Invalid);
        }
        // SAFETY: picolibc's own walk of the image's constructor table,
        // once, before any C code; `start` has registered the system.
        unsafe { __libc_init_array() };
        let module = load_module(name)?;
        if module.entry == 0 {
            return Err(Errno::Invalid);
        }
        // C's argv: the strings' addresses, then a null one.
        let mut argv = [core::ptr::null::<c_char>(); ARGS + 1];
        for (slot, arg) in argv.iter_mut().zip(args) {
            *slot = arg.as_ptr();
        }
        // SAFETY: the entry is an address the loader checked to be inside
        // the module it loaded and checked; the build put the module's
        // `int entry(int argc, char **argv)` there. `argv` lives until the
        // call returns and its strings for ever.
        let code = unsafe {
            let entry: unsafe extern "C" fn(c_int, *const *const c_char) -> c_int =
                core::mem::transmute(module.entry);
            entry(args.len() as c_int, argv.as_ptr())
        };
        // SAFETY: as in `run`.
        unsafe {
            let _ = fflush(core::ptr::addr_of!(stdout).read());
        }
        end();
        Ok(code)
    }

    /// `rp_module_load` (`ports/include/retropane.h`): the module's
    /// handle, or null with `errno`.
    ///
    /// # Safety
    /// `name` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rp_module_load(name: *const c_char) -> *mut c_void {
        if name.is_null() {
            fail(Errno::Invalid);
            return core::ptr::null_mut();
        }
        // SAFETY: the caller's contract: a NUL-terminated string at `name`.
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        match load_module(name) {
            Ok(m) => m.handle as *mut c_void,
            Err(e) => {
                fail(e);
                core::ptr::null_mut()
            }
        }
    }

    /// `rp_module_free`: 0, or -1 with `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_module_free(module: *mut c_void) -> c_int {
        match with(|s| s.module_free(module as usize)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_module_find`: the exported function's address, or null (not an
    /// error a program need read: `errno` is left alone).
    ///
    /// # Safety
    /// `name` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rp_module_find(
        module: *mut c_void,
        name: *const c_char,
    ) -> *mut c_void {
        if name.is_null() {
            return core::ptr::null_mut();
        }
        // SAFETY: the caller's contract: a NUL-terminated string at `name`.
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        with(|s| s.module_find(module as usize, name))
            .map_or(core::ptr::null_mut(), |at| at as *mut c_void)
    }

    /// `rp_module_name`: the name of the exported function at `function`
    /// (the module's own string), or null.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_module_name(
        module: *mut c_void,
        function: *const c_void,
    ) -> *const c_char {
        with(|s| s.module_name(module as usize, function as usize))
            .map_or(core::ptr::null(), |at| at as *const c_char)
    }

    /// `rp_module_error`: why the last module was refused.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_module_error() -> *const c_char {
        with(|s| Ok(s.module_reason())).map_or(c"no system is registered".as_ptr(), CStr::as_ptr)
    }

    /// Registers what answers the C library's calls and what `_exit` does.
    /// Before any C code runs.
    pub fn start(sys: &'static mut dyn Sys, exit: fn(i32) -> !) {
        SYS.with(|s| *s = Some(Held(sys)));
        EXIT.with(|e| *e = Some(exit));
    }

    /// The system the C calls reach, for its runner after the program
    /// has ended (a run's own measurements written to the card through
    /// the program's file table); `None` before [`start`].
    pub fn with_system<R>(f: impl FnOnce(&mut dyn Sys) -> R) -> Option<R> {
        SYS.with(|s| s.as_mut().map(|held| f(&mut *held.0)))
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

    /// `rp_big_alloc` (`ports/include/retropane.h`, R310): a large block
    /// from the heap's top region, or NULL (then `malloc`).
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_big_alloc(size: usize) -> *mut c_void {
        with(|s| Ok(s.big_alloc(size))).ok().flatten().map_or(core::ptr::null_mut(), |a| a as *mut c_void)
    }

    /// `rp_big_free`: 1 if `p` was a large block and is free now, else 0.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_big_free(p: *mut c_void) -> c_int {
        c_int::from(with(|s| Ok(s.big_free(p.addr()))).unwrap_or(false))
    }

    /// `rp_big_size`: the size of the large block at `p`, 0 if it is not one.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_big_size(p: *const c_void) -> usize {
        with(|s| Ok(s.big_size(p.addr()))).unwrap_or(0)
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

    /// `stat(2)`: the kind, size and time of what `path` names.
    ///
    /// # Safety
    /// `path` is a NUL-terminated string, `out` a writable `struct stat`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn stat(path: *const c_char, out: *mut CStat) -> c_int {
        if path.is_null() || out.is_null() || !out.is_aligned() {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: a NUL-terminated string at `path`.
        let name = unsafe { CStr::from_ptr(path) }.to_bytes();
        match with(|s| s.stat(name)) {
            Ok(st) => {
                // SAFETY: the caller's contract: a writable `struct stat`
                // at `out`, not null and aligned (checked), written whole.
                unsafe { out.write(CStat::of(&st)) };
                0
            }
            Err(e) => fail(e) as c_int,
        }
    }

    /// An open folder: picolibc's `DIR` is whatever its system makes it; a
    /// C program only holds the pointer.
    struct Dir {
        /// The handle, or -1 when this one is free.
        fd: c_int,
        entry: Dirent,
    }

    /// Folders a C program may have open at once (a game lists one at a
    /// time; the file table's handles are the real limit).
    const DIRS: usize = 4;
    static DIR_POOL: Global<[Dir; DIRS]> = Global::new(
        [const {
            Dir {
                fd: -1,
                entry: Dirent {
                    ino: 0,
                    kind: 0,
                    name: [0; NAME_MAX + 1],
                },
            }
        }; DIRS],
    );

    /// The pool's slot a `DIR *` points at, if it is one of ours and open.
    fn dir_slot(dir: *mut c_void) -> Option<usize> {
        DIR_POOL.with(|pool| {
            pool.iter().position(|d| {
                core::ptr::eq(core::ptr::from_ref(d).cast::<c_void>(), dir) && d.fd >= 0
            })
        })
    }

    /// `opendir(3)`: null with `errno` if the folder cannot be opened or
    /// four are open already.
    ///
    /// # Safety
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut c_void {
        if path.is_null() {
            fail(Errno::Invalid);
            return core::ptr::null_mut();
        }
        let Some(k) = DIR_POOL.with(|pool| pool.iter().position(|d| d.fd < 0)) else {
            fail(Errno::NoHandle);
            return core::ptr::null_mut();
        };
        // SAFETY: the caller's contract: a NUL-terminated string at `path`.
        let name = unsafe { CStr::from_ptr(path) }.to_bytes();
        match with(|s| s.dir_open(name)) {
            Ok(fd) => DIR_POOL.with(|pool| {
                pool[k].fd = fd;
                core::ptr::from_mut(&mut pool[k]).cast::<c_void>()
            }),
            Err(e) => {
                fail(e);
                core::ptr::null_mut()
            }
        }
    }

    /// `readdir(3)`: the next entry (good until the next call on this
    /// folder), or null at its end (`errno` untouched) or on an error.
    #[unsafe(no_mangle)]
    pub extern "C" fn readdir(dir: *mut c_void) -> *mut Dirent {
        let Some(k) = dir_slot(dir) else {
            fail(Errno::BadHandle);
            return core::ptr::null_mut();
        };
        DIR_POOL.with(|pool| {
            let d = &mut pool[k];
            match with(|s| s.dir_next(d.fd, &mut d.entry.name[..NAME_MAX])) {
                Ok(Some((n, folder))) => {
                    d.entry.name[n.min(NAME_MAX)] = 0;
                    d.entry.kind = if folder { 3 } else { 6 };
                    core::ptr::from_mut(&mut d.entry)
                }
                Ok(None) => core::ptr::null_mut(),
                Err(e) => {
                    fail(e);
                    core::ptr::null_mut()
                }
            }
        })
    }

    /// `closedir(3)`.
    #[unsafe(no_mangle)]
    pub extern "C" fn closedir(dir: *mut c_void) -> c_int {
        let Some(k) = dir_slot(dir) else {
            return fail(Errno::BadHandle) as c_int;
        };
        let fd = DIR_POOL.with(|pool| core::mem::replace(&mut pool[k].fd, -1));
        match with(|s| s.close(fd)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `mkdir(2)`; the mode is not used (no permissions on the card).
    ///
    /// # Safety
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn mkdir(path: *const c_char, _mode: u32) -> c_int {
        if path.is_null() {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: a NUL-terminated string at `path`.
        let name = unsafe { CStr::from_ptr(path) }.to_bytes();
        match with(|s| s.mkdir(name)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rename(2)`.
    ///
    /// # Safety
    /// `from` and `to` are NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rename(from: *const c_char, to: *const c_char) -> c_int {
        if from.is_null() || to.is_null() {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: NUL-terminated strings at both.
        let (from, to) = unsafe {
            (
                CStr::from_ptr(from).to_bytes(),
                CStr::from_ptr(to).to_bytes(),
            )
        };
        match with(|s| s.rename(from, to)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `dup(2)`.
    #[unsafe(no_mangle)]
    pub extern "C" fn dup(fd: c_int) -> c_int {
        match with(|s| s.dup(fd)) {
            Ok(new) => new,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `fsync(2)`.
    #[unsafe(no_mangle)]
    pub extern "C" fn fsync(fd: c_int) -> c_int {
        match with(|s| s.sync(fd)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `getcwd(3)`: the folder relative paths start from; "/", the card's
    /// root, until the program changes it (`ERANGE` is not among this
    /// boundary's numbers: a buffer too small is `EINVAL`).
    ///
    /// # Safety
    /// `buf` points at `size` writable bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn getcwd(buf: *mut c_char, size: usize) -> *mut c_char {
        if buf.is_null() || size < 2 || size > isize::MAX as usize {
            fail(Errno::Invalid);
            return core::ptr::null_mut();
        }
        // SAFETY: the caller's contract: `size` writable bytes at `buf`,
        // not null (checked), the C program's alone while this call runs.
        let into = unsafe { core::slice::from_raw_parts_mut(buf.cast::<u8>(), size) };
        let (path, end) = into.split_at_mut(size - 1);
        match with(|s| s.cwd(path).ok_or(Errno::Invalid)) {
            Ok(n) => {
                // The NUL: right after the path, or in the byte kept back.
                match path.get_mut(n) {
                    Some(b) => *b = 0,
                    None => end[0] = 0,
                }
                buf
            }
            Err(e) => {
                fail(e);
                core::ptr::null_mut()
            }
        }
    }

    /// `chdir(2)`.
    ///
    /// # Safety
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
        if path.is_null() {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: a NUL-terminated string at `path`.
        let name = unsafe { CStr::from_ptr(path) }.to_bytes();
        match with(|s| s.chdir(name)) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `execve(2)`: a program cannot become another here (one image runs
    /// at a time and the loader starts it): always refused.
    #[unsafe(no_mangle)]
    pub extern "C" fn execve(
        _path: *const c_char,
        _argv: *const c_void,
        _envp: *const c_void,
    ) -> c_int {
        fail(Errno::NoSys) as c_int
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

    /// `rp_gfx_open` (`ports/include/retropane.h`): 0 with the screen in
    /// `*g`, or -1 with `errno`.
    ///
    /// # Safety
    /// `g` points at a writable `struct rp_gfx`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rp_gfx_open(g: *mut Gfx) -> c_int {
        if g.is_null() || !g.is_aligned() {
            return fail(Errno::Invalid) as c_int;
        }
        match with(|s| s.gfx_open()) {
            Ok(info) => {
                // SAFETY: the caller's contract: a writable `struct
                // rp_gfx` at `g`, not null and aligned (both checked),
                // written whole.
                unsafe { g.write(info) };
                0
            }
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_gfx_frame`: 0, or -1 with `errno`.
    ///
    /// # Safety
    /// `words` points at `count` readable 32-bit words.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rp_gfx_frame(words: *const u32, count: usize) -> c_int {
        if words.is_null() || !words.is_aligned() || count > isize::MAX as usize / 4 {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: `count` readable words at `words`,
        // not null and aligned (both checked), not changed while this call
        // runs (one core, one C thread).
        let words = unsafe { core::slice::from_raw_parts(words, count) };
        match with(|s| s.gfx_frame(&[words])) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_gfx_frame_parts`'s part (`struct rp_gfx_part`).
    #[repr(C)]
    pub struct GfxPart {
        words: *const u32,
        count: usize,
    }

    /// Parts a frame may have (`RP_GFX_PARTS`).
    pub const GFX_PARTS: usize = 32;

    /// `rp_gfx_frame_parts`: 0, or -1 with `errno`. Kit 51 (ref_rp's
    /// second pass was copied after the first, ~1-2% of Half-Life's play,
    /// run 49): the parts go to the feed as they lie, no copy to join them.
    ///
    /// # Safety
    /// `parts` points at `n` readable parts, each at `count` readable
    /// 32-bit words.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn rp_gfx_frame_parts(parts: *const GfxPart, n: usize) -> c_int {
        if parts.is_null() || !parts.is_aligned() || n > GFX_PARTS {
            return fail(Errno::Invalid) as c_int;
        }
        // SAFETY: the caller's contract: `n` readable parts, not null and
        // aligned (both checked), not changed while this call runs.
        let parts = unsafe { core::slice::from_raw_parts(parts, n) };
        let mut list: [&[u32]; GFX_PARTS] = [&[]; GFX_PARTS];
        for (slot, p) in list.iter_mut().zip(parts) {
            if p.count == 0 {
                continue;
            }
            if p.words.is_null() || !p.words.is_aligned() || p.count > isize::MAX as usize / 4 {
                return fail(Errno::Invalid) as c_int;
            }
            // SAFETY: the caller's contract, as `rp_gfx_frame`'s, a part
            // at a time (not null, aligned and sized: checked).
            *slot = unsafe { core::slice::from_raw_parts(p.words, p.count) };
        }
        match with(|s| s.gfx_frame(&list[..n])) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_gfx_wait`: 0, or -1 with `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_gfx_wait() -> c_int {
        match with(|s| s.gfx_wait()) {
            Ok(()) => 0,
            Err(e) => fail(e) as c_int,
        }
    }

    /// `rp_gfx_close`: 0, or -1 with `errno`.
    #[unsafe(no_mangle)]
    pub extern "C" fn rp_gfx_close() -> c_int {
        match with(|s| s.gfx_close()) {
            Ok(()) => 0,
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

    /// `clock_gettime(2)`: every clock is the program's one clock
    /// ([`rp_time_us`]: it never goes back, which is all `CLOCK_MONOTONIC`
    /// promises, and the date is not the port layer's to know yet).
    /// picolibc's `struct timespec` is a 64-bit `time_t` and a `long`, as
    /// [`Timeval`] is laid out.
    ///
    /// # Safety
    /// `ts` is null or points at a writable `struct timespec`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn clock_gettime(_clock: c_int, ts: *mut Timeval) -> c_int {
        if ts.is_null() {
            return fail(Errno::Invalid) as c_int;
        }
        match with(|s| Ok(s.now_us())) {
            Ok(us) => {
                let (sec, usec) = super::split_us(us);
                // SAFETY: the caller's contract: a writable `struct
                // timespec` at `ts`, not null (checked), written whole.
                unsafe {
                    ts.write(Timeval {
                        sec,
                        usec: usec * 1_000,
                    });
                }
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
pub use face::{ARGS, run, run_module, runner_table, start, with_system};

#[cfg(test)]
mod big_tests {
    use super::{BIG_BLOCKS, Break};

    #[test]
    fn large_blocks_come_from_the_top_and_the_break_cannot_pass_them() {
        let mut h = Break::new(0x1000, 0x11000);
        let a = h.big_alloc(0x4000).unwrap();
        assert_eq!(a, 0x11000 - 0x4000);
        assert_eq!(h.big_size(a), 0x4000);
        assert_eq!(h.used(), 0x4000);
        // the break may grow to the region and not past it
        assert!(h.grow(0xC000).is_some());
        assert!(h.grow(1).is_none());
        // and the region may not grow down past the break
        assert!(h.big_alloc(32).is_none());
    }

    #[test]
    fn a_freed_block_joins_its_neighbours_and_the_bottom_goes_back() {
        let mut h = Break::new(0, 0x10000);
        let a = h.big_alloc(0x1000).unwrap();
        let b = h.big_alloc(0x1000).unwrap();
        let c = h.big_alloc(0x1000).unwrap();
        assert!(c < b && b < a);
        assert!(h.big_free(b));
        assert_eq!(h.big_free_bytes(), 0x1000);
        assert!(!h.big_free(b), "a block freed twice is refused");
        assert!(h.big_free(a));
        // a and b joined into one hole of 0x2000 above c
        assert_eq!(h.big_free_bytes(), 0x2000);
        let d = h.big_alloc(0x2000).unwrap();
        assert_eq!(d, b, "the joined hole takes a block of both sizes");
        assert!(h.big_free(d));
        assert!(h.big_free(c));
        // all free: the whole region back to the break
        assert_eq!(h.used(), 0);
        assert_eq!(h.big_free_bytes(), 0);
        assert!(h.grow(0x10000).is_some());
    }

    #[test]
    fn best_fit_splits_the_smallest_hole_and_keeps_alignment() {
        let mut h = Break::new(0, 0x100000);
        let big = h.big_alloc(0x8000).unwrap();
        let _keep = h.big_alloc(0x100).unwrap();
        let small = h.big_alloc(0x1000).unwrap();
        let _keep2 = h.big_alloc(0x100).unwrap();
        h.big_free(big);
        h.big_free(small);
        let x = h.big_alloc(0x800).unwrap();
        assert!(x >= small && x < small + 0x1000, "the smaller hole was used");
        assert_eq!(x % 32, 0);
        let one = h.big_alloc(1).unwrap();
        assert_eq!(h.big_size(one), 32);
    }

    #[test]
    fn a_full_table_or_a_bad_address_is_refused_not_a_crash() {
        let mut h = Break::new(0, 1 << 24);
        let mut got = 0;
        while h.big_alloc(64).is_some() {
            got += 1;
        }
        assert_eq!(got, BIG_BLOCKS);
        assert!(!h.big_free(12345));
        assert_eq!(h.big_size(12345), 0);
        assert!(h.big_alloc(usize::MAX).is_none());
    }
}

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
    fn stat_and_dirent_are_the_c_library_s_structures() {
        // picolibc 1.8.12's sys/stat.h on a 32-bit ARM (64-bit dev_t,
        // ino_t, nlink_t, off_t, blkcnt_t and time_t; 32-bit long).
        assert_eq!(core::mem::size_of::<CStat>(), 120);
        assert_eq!(core::mem::offset_of!(CStat, mode), 16);
        assert_eq!(core::mem::offset_of!(CStat, size), 48);
        assert_eq!(core::mem::offset_of!(CStat, mtime), 88);
        // sys/dirent.h: a 64-bit ino_t, d_type, then the name.
        assert_eq!(core::mem::offset_of!(Dirent, kind), 8);
        assert_eq!(core::mem::offset_of!(Dirent, name), 9);
        assert_eq!(NAME_MAX, 255);
        let f = CStat::of(&FileStat {
            folder: false,
            size: 1_025,
            modified: 315_532_800,
        });
        // S_ISREG and S_ISDIR test these bits (S_IFMT 0170000).
        assert_eq!(f.mode & 0o170_000, 0o100_000);
        assert_eq!((f.size, f.blocks, f.mtime), (1_025, 3, (315_532_800, 0)));
        let d = CStat::of(&FileStat {
            folder: true,
            size: 0,
            modified: 0,
        });
        assert_eq!(d.mode & 0o170_000, 0o040_000);
        // A size past what off_t holds is its largest, not a negative.
        let huge = CStat::of(&FileStat {
            folder: false,
            size: u64::MAX,
            modified: 0,
        });
        assert_eq!(huge.size, i64::MAX);
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
