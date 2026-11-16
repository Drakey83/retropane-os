//! Files as a C program uses them: open by path, read, seek, write, list a
//! folder, over the card's volume.

use rp_fat::node::{self, Node, ROOT, Stamp};
use rp_fat::{AnyVolume, Error as FatError};
use rp_hal_api::BlockDevice;

use crate::names::{Brief, Names};

/// A handle: an index into the table, as C's `int fd`.
pub type Fd = u16;

/// Why a call failed (the C face turns these into `errno`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No such file or folder.
    NotFound,
    /// The path is not one (empty, a name too long, a file used as a
    /// folder).
    BadPath,
    /// Not a handle, or not one of this kind (a folder read as a file).
    BadHandle,
    /// Every handle is in use.
    NoHandle,
    /// A file is already being written (one at a time), or the written
    /// file outgrew the room given for it.
    NoRoom,
    /// The card or its file system.
    Card(FatError),
}

impl From<FatError> for Error {
    fn from(e: FatError) -> Self {
        match e {
            FatError::NotFound => Self::NotFound,
            e => Self::Card(e),
        }
    }
}

/// Where a seek counts from (C's `SEEK_SET`, `SEEK_CUR`, `SEEK_END`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whence {
    /// From the file's start.
    Start,
    /// From where it is.
    Here,
    /// From the file's end.
    End,
}

/// A folder's entry, as [`Files::next`] gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Bytes of its name written into the caller's buffer.
    pub name_len: usize,
    /// A folder.
    pub folder: bool,
    /// Its size in bytes.
    pub size: u64,
}

/// What a path names, as C's `stat` asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    /// A folder.
    pub folder: bool,
    /// Its size in bytes (0 for a folder).
    pub size: u64,
    /// When it was last changed, if the card says.
    pub modified: Option<Stamp>,
}

/// A card's date and time as seconds since 1970 (as written: the card's
/// own clock, no time zone applied), for C's `st_mtime`. Days by the
/// civil-calendar count; a date before 1970 or a month or day that is none
/// gives 0.
#[must_use]
pub fn unix_seconds(s: &Stamp) -> i64 {
    if s.year < 1970 || !(1..=12).contains(&s.month) || !(1..=31).contains(&s.day) {
        return 0;
    }
    // Years begin in March, so the leap day is the year's last.
    let (y, m) = if s.month <= 2 {
        (i64::from(s.year) - 1, i64::from(s.month) + 9)
    } else {
        (i64::from(s.year), i64::from(s.month) - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + i64::from(s.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + i64::from(s.hour) * 3_600 + i64::from(s.minute) * 60 + i64::from(s.second)
}

/// A name's most UTF-16 units kept for the file being written.
const NAME: usize = 64;

// (No heap to box the larger ones into: a slot is the size of its
// largest, a file's node with its name, about 600 bytes.)
#[allow(clippy::large_enum_variant)]
enum Slot {
    Free,
    /// A file being read: where it is and how far the reading has got.
    File {
        node: Node,
        at: u64,
    },
    /// A folder being listed: from the card in its order (`cursor`), or
    /// from the names remembered in theirs (`sorted`), after `last`.
    Dir {
        dir: u64,
        cursor: u64,
        sorted: bool,
        last: Node,
    },
    /// A file being written: its folder, its name, where its bytes start
    /// in the table's room, how far the writing has got and how long it
    /// is.
    Out {
        dir: u64,
        name: [u16; NAME],
        name_len: usize,
        start: usize,
        at: usize,
        len: usize,
    },
}

/// The table of open files over one volume: `N` handles, `scratch` for the
/// file system's sector work (the bigger, the longer its reads in one
/// command), `room` for the files being written: each is kept whole
/// there until it is closed, packed one after another, and one that grows
/// pushes those after it along (a game keeps a log or a recording open
/// while it writes a save; the first version took one written file at a
/// time, and Half-Life's first map could save nothing: 2026-11-15).
pub struct Files<'a, D: BlockDevice, const N: usize> {
    vol: AnyVolume<D>,
    slots: [Slot; N],
    scratch: &'a mut [u8],
    room: &'a mut [u8],
    /// The folder a path that does not start with a slash is walked from
    /// (C's current directory): the root until [`Files::set_current`].
    current: u64,
    /// The names of the folders looked into, where the image gave room
    /// for them ([`Files::remember`]): a look-up then costs no card read.
    names: Option<Names<'a>>,
}

/// Whether `path` starts at the card's root whatever the current folder.
fn rooted(path: &[u8]) -> bool {
    matches!(path.first(), Some(b'/' | b'\\'))
}

/// A path's next name: the units up to a slash, either way (C code says
/// `valve/maps`, old code `valve\maps`), and the rest.
fn split(path: &[u8]) -> (&[u8], &[u8]) {
    let path = match path.iter().position(|&c| c != b'/' && c != b'\\') {
        Some(k) => &path[k..],
        None => &[],
    };
    match path.iter().position(|&c| c == b'/' || c == b'\\') {
        Some(k) => (&path[..k], &path[k + 1..]),
        None => (path, &[]),
    }
}

/// `name` (bytes; above ASCII as Latin-1) as UTF-16 units: how many, or
/// `None` when it does not fit.
fn units(name: &[u8], out: &mut [u16]) -> Option<usize> {
    if name.len() > out.len() {
        return None;
    }
    for (o, &c) in out.iter_mut().zip(name) {
        *o = u16::from(c);
    }
    Some(name.len())
}

impl<'a, D: BlockDevice, const N: usize> Files<'a, D, N> {
    /// The table over `vol`.
    pub fn new(vol: AnyVolume<D>, scratch: &'a mut [u8], room: &'a mut [u8]) -> Self {
        Self {
            vol,
            slots: [const { Slot::Free }; N],
            scratch,
            room,
            current: ROOT,
            // Under test, every table is also run with the names
            // remembered, in the room the test's loop says
            // (`tests::each`): all of this crate's file tests then hold
            // with them, without them, and with too little room for them.
            #[cfg(test)]
            names: match tests::NAMES_ROOM.get() {
                0 => None,
                n => Some(Names::new(Vec::leak(vec![0u8; n]))),
            },
            #[cfg(not(test))]
            names: None,
        }
    }

    /// Keeps the names of every folder looked into in `names`, so that
    /// what a path names is answered without the card from the second
    /// time on (a folder that does not fit there is walked as before).
    /// Only this table may change the card while it does: a folder it
    /// changes is forgotten and listed again.
    pub fn remember(&mut self, names: Names<'a>) {
        self.names = Some(names);
    }

    /// How much of the names' room is in use, if there is one.
    #[must_use]
    pub fn remembered(&self) -> Option<crate::names::Used> {
        self.names.as_ref().map(Names::used)
    }

    /// `dir`'s names are about to change: forgotten.
    fn forget(&mut self, dir: u64) {
        if let Some(n) = self.names.as_mut() {
            n.forget(dir);
        }
    }

    /// A folder is about to move or go: every name is forgotten (those
    /// kept under its place would answer for whatever takes it).
    fn forget_all(&mut self) {
        if let Some(n) = self.names.as_mut() {
            n.clear();
        }
    }

    /// Lists `dir` into the names' room: whether it is remembered now.
    fn list(&mut self, dir: u64) -> Result<bool, Error> {
        let Some(names) = self.names.as_mut() else {
            return Ok(false);
        };
        if names.listed(dir) {
            return Ok(true);
        }
        if !names.begin(dir) {
            return Ok(false);
        }
        let mut node = Node::default();
        let mut cursor = 0;
        loop {
            match self.vol.node_child(dir, &mut cursor, &mut node) {
                Ok(true) => {
                    if !names.add(&node) {
                        return Ok(false);
                    }
                }
                Ok(false) => {
                    names.keep();
                    return Ok(true);
                }
                Err(e) => {
                    names.abandon();
                    return Err(e.into());
                }
            }
        }
    }

    /// What the child of folder `dir` named `name` is, any case: from
    /// the names remembered, or by walking the folder on the card.
    fn brief(&mut self, dir: u64, name: &[u8]) -> Result<Brief, Error> {
        let mut want = [0u16; node::NAME_UNITS];
        let n = units(name, &mut want).ok_or(Error::BadPath)?;
        if self.list(dir)?
            && let Some(known) = self.names.as_ref().and_then(|c| c.get(dir, &want[..n]))
        {
            return known.ok_or(Error::NotFound);
        }
        let mut node = Node::default();
        self.walk(dir, &want[..n], &mut node)?;
        Ok(Brief::of(&node))
    }

    /// Where the written files' bytes end in the room.
    fn out_end(&self) -> usize {
        self.slots
            .iter()
            .map(|s| match s {
                Slot::Out { start, len, .. } => start + len,
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    /// Makes the written file in slot `k` `new_len` bytes long (longer or
    /// shorter): the files after it in the room move with its end.
    fn resize_out(&mut self, k: usize, new_len: usize) -> Result<(), Error> {
        let Some(Slot::Out { start, len, .. }) = self.slots.get(k) else {
            return Err(Error::BadHandle);
        };
        let (tail, end) = (start + len, self.out_end());
        let new_tail = start + new_len;
        if new_tail > tail && end + (new_tail - tail) > self.room.len() {
            return Err(Error::NoRoom);
        }
        self.room.copy_within(tail..end, new_tail);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if let Slot::Out { start, len, .. } = slot {
                if i == k {
                    *len = new_len;
                } else if *start >= tail {
                    *start = *start - tail + new_tail;
                }
            }
        }
        Ok(())
    }

    /// The folder `path` names: the current one for an empty path or `.`,
    /// the root for a slash alone.
    fn folder(&mut self, path: &[u8]) -> Result<u64, Error> {
        let (dir, name) = self.parent(path)?;
        if name.is_empty() || name == b"." {
            return Ok(dir);
        }
        let b = self.brief(dir, name)?;
        if b.folder {
            Ok(b.loc)
        } else {
            Err(Error::BadPath)
        }
    }

    /// Makes the folder at `path` the one relative paths start from (C's
    /// `chdir`): a game's file system names its files from its own folder.
    ///
    /// # Errors
    /// No such folder, or the card.
    pub fn set_current(&mut self, path: &[u8]) -> Result<(), Error> {
        self.current = self.folder(path)?;
        Ok(())
    }

    /// The volume back (the card handed on).
    pub fn into_volume(self) -> AnyVolume<D> {
        self.vol
    }

    /// The child of folder `dir` named `want`, any case, found by
    /// reading the folder's entries from the card.
    fn walk(&mut self, dir: u64, want: &[u16], out: &mut Node) -> Result<(), Error> {
        let mut cursor = 0;
        while self.vol.node_child(dir, &mut cursor, out)? {
            if node::same_name(out.name(), want) {
                return Ok(());
            }
        }
        Err(Error::NotFound)
    }

    /// The child of folder `dir` named `name`, any case, whole (what
    /// reading or changing it needs): its entry read from where the
    /// names remembered say it is, or the folder walked.
    fn child(&mut self, dir: u64, name: &[u8], out: &mut Node) -> Result<(), Error> {
        let mut want = [0u16; node::NAME_UNITS];
        let n = units(name, &mut want).ok_or(Error::BadPath)?;
        if self.list(dir)?
            && let Some(known) = self.names.as_ref().and_then(|c| c.get(dir, &want[..n]))
        {
            let b = known.ok_or(Error::NotFound)?;
            return Ok(self.vol.node_stat(b.loc, out)?);
        }
        self.walk(dir, &want[..n], out)
    }

    /// The folder a path's last name lies in, and that name: every name
    /// before it walked from the root, or from the current folder if the
    /// path does not start with a slash (`.` skipped; `..` is not a name
    /// a game's files use, and is refused).
    fn parent<'p>(&mut self, path: &'p [u8]) -> Result<(u64, &'p [u8]), Error> {
        let mut dir = if rooted(path) { ROOT } else { self.current };
        let (mut name, mut rest) = split(path);
        loop {
            if name == b".." {
                return Err(Error::BadPath);
            }
            let (next, after) = split(rest);
            if next.is_empty() {
                return Ok((dir, name));
            }
            if name != b"." {
                let b = self.brief(dir, name)?;
                if !b.folder {
                    return Err(Error::BadPath);
                }
                dir = b.loc;
            }
            (name, rest) = (next, after);
        }
    }

    /// What `path` names.
    fn find(&mut self, path: &[u8], out: &mut Node) -> Result<(), Error> {
        let (dir, name) = self.parent(path)?;
        if name.is_empty() || name == b"." {
            return Err(Error::BadPath);
        }
        self.child(dir, name, out)
    }

    fn free_slot(&self) -> Result<usize, Error> {
        self.slots
            .iter()
            .position(|s| matches!(s, Slot::Free))
            .ok_or(Error::NoHandle)
    }

    /// Opens the file at `path` for reading.
    ///
    /// # Errors
    /// No such file, a folder, no handle left, or the card.
    pub fn open(&mut self, path: &[u8]) -> Result<Fd, Error> {
        let k = self.free_slot()?;
        let mut node = Node::default();
        self.find(path, &mut node)?;
        if node.folder {
            return Err(Error::BadPath);
        }
        self.slots[k] = Slot::File { node, at: 0 };
        Ok(k as Fd)
    }

    /// Starts the file at `path` anew for writing (its folder must be
    /// there). Its bytes are kept in the table's room and reach the card
    /// when it is closed; until then the old file, if any, is whole.
    ///
    /// # Errors
    /// No such folder, a name too long, a file already being written, no
    /// handle left, or the card.
    pub fn create(&mut self, path: &[u8]) -> Result<Fd, Error> {
        let k = self.free_slot()?;
        let (dir, last) = self.parent(path)?;
        let mut name = [0u16; NAME];
        let name_len = units(last, &mut name).ok_or(Error::BadPath)?;
        if !node::valid_name(&name[..name_len]) {
            return Err(Error::BadPath);
        }
        self.slots[k] = Slot::Out {
            dir,
            name,
            name_len,
            start: self.out_end(),
            at: 0,
            len: 0,
        };
        Ok(k as Fd)
    }

    /// Opens the file at `path` to be changed (C's `"r+"` and `"a"`): what
    /// is on the card is read into the table's room, the changes are made
    /// there, and the whole file goes back onto the card when it is closed
    /// (so a power cut leaves the old file or the new one, as with
    /// [`Self::create`]). `at_end`: every write goes after what is there.
    /// `make`: a file that is not there starts empty.
    ///
    /// # Errors
    /// No such file (and not `make`), a folder, a file already being
    /// written, a file bigger than the room, no handle left, or the card.
    pub fn reopen(&mut self, path: &[u8], at_end: bool, make: bool) -> Result<Fd, Error> {
        let fd = self.create(path)?;
        let (dir, last) = match self.parent(path) {
            Ok(p) => p,
            Err(e) => {
                self.slots[usize::from(fd)] = Slot::Free;
                return Err(e);
            }
        };
        let mut node = Node::default();
        let had = match self.child(dir, last, &mut node) {
            Ok(()) if node.folder => Err(Error::BadPath),
            Ok(()) => usize::try_from(node.size)
                .ok()
                .filter(|n| *n <= self.room.len() - self.out_end())
                .ok_or(Error::NoRoom),
            Err(Error::NotFound) if make => Ok(0),
            Err(e) => Err(e),
        };
        let size = match had {
            Ok(n) => n,
            Err(e) => {
                self.slots[usize::from(fd)] = Slot::Free;
                return Err(e);
            }
        };
        // Its room first (it is the last in the room: nothing moves).
        let start = self.out_end();
        if let Err(e) = self.resize_out(usize::from(fd), size) {
            self.slots[usize::from(fd)] = Slot::Free;
            return Err(e);
        }
        let mut got = 0;
        while got < size {
            match self.vol.node_read(
                &node,
                got as u64,
                &mut self.room[start + got..start + size],
                self.scratch,
            ) {
                Ok(n) if n > 0 => got += n,
                Ok(_) => break,
                Err(e) => {
                    self.slots[usize::from(fd)] = Slot::Free;
                    return Err(e.into());
                }
            }
        }
        // A file that read short is as long as what was read.
        let _ = self.resize_out(usize::from(fd), got);
        if let Slot::Out { at, .. } = &mut self.slots[usize::from(fd)] {
            *at = if at_end { got } else { 0 };
        }
        Ok(fd)
    }

    /// A second handle on what `fd` has open, with a place of its own
    /// starting where `fd` is (C's `dup`, but for the shared place: the
    /// engine that needs this seeks each handle before it reads).
    ///
    /// # Errors
    /// Not a handle, the file being written (it has one), or no handle
    /// left.
    pub fn dup(&mut self, fd: Fd) -> Result<Fd, Error> {
        let k = self.free_slot()?;
        let copy = match self.slots.get(usize::from(fd)) {
            Some(Slot::File { node, at }) => Slot::File {
                node: *node,
                at: *at,
            },
            Some(Slot::Dir {
                dir,
                cursor,
                sorted,
                last,
            }) => Slot::Dir {
                dir: *dir,
                cursor: *cursor,
                sorted: *sorted,
                last: *last,
            },
            _ => return Err(Error::BadHandle),
        };
        self.slots[k] = copy;
        Ok(k as Fd)
    }

    /// What `path` names (`""` or `"/"`: the root, a folder).
    ///
    /// # Errors
    /// Nothing of that name, or the card.
    pub fn stat(&mut self, path: &[u8]) -> Result<Stat, Error> {
        if split(path).0.is_empty() {
            return Ok(Stat {
                folder: true,
                size: 0,
                modified: None,
            });
        }
        let (dir, name) = self.parent(path)?;
        if name.is_empty() || name == b"." {
            return Err(Error::BadPath);
        }
        let b = self.brief(dir, name)?;
        Ok(Stat {
            folder: b.folder,
            size: if b.folder { 0 } else { b.size },
            modified: b.modified,
        })
    }

    /// Gives the file or folder at `from` the path `to` (its folder must
    /// be there). A file already at `to` makes way, as C's `rename` says.
    ///
    /// # Errors
    /// Nothing at `from`, no such folder for `to`, a bad name, a folder
    /// already at `to`, or the card.
    pub fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Error> {
        let (fdir, fname) = self.parent(from)?;
        let mut node = Node::default();
        self.child(fdir, fname, &mut node)?;
        let (tdir, tname) = self.parent(to)?;
        let mut name = [0u16; NAME];
        let n = units(tname, &mut name).ok_or(Error::BadPath)?;
        if !node::valid_name(&name[..n]) {
            return Err(Error::BadPath);
        }
        let mut there = Node::default();
        match self.child(tdir, tname, &mut there) {
            // Itself under another case: nothing to make way.
            Ok(()) if there.loc == node.loc => {}
            Ok(()) if there.folder => return Err(Error::BadPath),
            Ok(()) => {
                self.forget(tdir);
                self.vol.node_delete(there.loc)?;
            }
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        if node.folder {
            self.forget_all();
        } else {
            self.forget(fdir);
            self.forget(tdir);
        }
        let mut loc = node.loc;
        let mut dir = fdir;
        if tdir != fdir {
            loc = self.vol.node_move(fdir, loc, tdir, self.scratch)?;
            dir = tdir;
        }
        if !(tdir != fdir && node.name() == &name[..n]) {
            self.vol.node_rename(dir, loc, &name[..n], self.scratch)?;
        }
        Ok(())
    }

    /// Opens the folder at `path` for listing (`""` or `"/"`: the root).
    ///
    /// # Errors
    /// No such folder, no handle left, or the card.
    pub fn open_dir(&mut self, path: &[u8]) -> Result<Fd, Error> {
        let k = self.free_slot()?;
        let dir = self.folder(path)?;
        self.slots[k] = Slot::Dir {
            dir,
            cursor: 0,
            sorted: false,
            last: Node::default(),
        };
        Ok(k as Fd)
    }

    /// The folder's next entry, its name into `name` (cut to its length;
    /// units above a byte as `?`); `None` at its end.
    ///
    /// # Errors
    /// Not a folder's handle, or the card.
    pub fn next(&mut self, fd: Fd, name: &mut [u8]) -> Result<Option<Entry>, Error> {
        let k = usize::from(fd);
        let Some(&Slot::Dir {
            dir,
            mut cursor,
            mut sorted,
            last,
        }) = self.slots.get(k)
        else {
            return Err(Error::BadHandle);
        };
        // At its start, a folder whose names can be kept is listed from
        // them: one walk of the card however often it is listed (R281).
        if cursor == 0 && !sorted && self.list(dir)? {
            sorted = true;
        }
        let mut node = Node::default();
        let more = if sorted {
            let after = (cursor > 0).then(|| last.name());
            cursor = 1;
            self.sorted_next(dir, after, &mut node)?
        } else {
            self.vol.node_child(dir, &mut cursor, &mut node)?
        };
        if let Some(Slot::Dir {
            cursor: c,
            sorted: s,
            last: l,
            ..
        }) = self.slots.get_mut(k)
        {
            (*c, *s) = (cursor, sorted);
            if more {
                *l = node;
            }
        }
        if !more {
            return Ok(None);
        }
        let units = node.name();
        let n = units.len().min(name.len());
        for (o, &u) in name.iter_mut().zip(units) {
            *o = u8::try_from(u).unwrap_or(b'?');
        }
        Ok(Some(Entry {
            name_len: n,
            folder: node.folder,
            size: node.size,
        }))
    }

    /// The first child of `dir` after `last` in the names' order
    /// ([`crate::names::order`]), into `out`: from the names remembered, or
    /// when they no longer hold the folder and cannot again (it changed
    /// while being listed and the room is full), by walking the card for
    /// it, each entry a whole walk: never a name twice or one left out.
    fn sorted_next(&mut self, dir: u64, last: Option<&[u16]>, out: &mut Node) -> Result<bool, Error> {
        if self.list(dir)?
            && let Some(names) = self.names.as_ref()
            && let Some(next) = names.after(dir, last)
        {
            return Ok(next.is_some_and(|c| {
                names.fill(c, out);
                true
            }));
        }
        let (mut cursor, mut node, mut found) = (0, Node::default(), false);
        while self.vol.node_child(dir, &mut cursor, &mut node)? {
            let later = last.is_none_or(|l| crate::names::order(node.name(), l).is_gt());
            if later && (!found || crate::names::order(node.name(), out.name()).is_lt()) {
                *out = node;
                found = true;
            }
        }
        Ok(found)
    }

    /// Reads into `buf` from where the file is: the bytes read (0 at its
    /// end).
    ///
    /// # Errors
    /// Not a file's handle, or the card.
    pub fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Error> {
        match self.slots.get_mut(usize::from(fd)) {
            Some(Slot::File { node, at }) => {
                let left = node.size.saturating_sub(*at);
                let n = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
                if n == 0 {
                    return Ok(0);
                }
                let got = self.vol.node_read(node, *at, &mut buf[..n], self.scratch)?;
                *at += got as u64;
                Ok(got)
            }
            // What was written so far reads back (a save checked before
            // it is closed).
            Some(Slot::Out { start, at, len, .. }) => {
                let n = buf.len().min(*len - *at);
                buf[..n].copy_from_slice(&self.room[*start + *at..*start + *at + n]);
                *at += n;
                Ok(n)
            }
            _ => Err(Error::BadHandle),
        }
    }

    /// Writes `data` where the written file is: the bytes taken.
    ///
    /// # Errors
    /// Not the written file's handle, or its room is full (nothing taken).
    pub fn write(&mut self, fd: Fd, data: &[u8]) -> Result<usize, Error> {
        let k = usize::from(fd);
        let Some(Slot::Out { at, len, .. }) = self.slots.get(k) else {
            return Err(Error::BadHandle);
        };
        let (from, len) = (*at, *len);
        let end = from.checked_add(data.len()).ok_or(Error::NoRoom)?;
        if end > len {
            self.resize_out(k, end)?;
        }
        if let Some(Slot::Out { start, at, .. }) = self.slots.get_mut(k) {
            self.room[*start + from..*start + end].copy_from_slice(data);
            *at = end;
        }
        Ok(data.len())
    }

    /// Moves to `offset` from `whence`: where the file is then (a read
    /// file may sit past its end and reads nothing there; a written file
    /// stays inside what was written).
    ///
    /// # Errors
    /// Not a file's handle, or a place before the start.
    pub fn seek(&mut self, fd: Fd, offset: i64, whence: Whence) -> Result<u64, Error> {
        let (at, size) = match self.slots.get(usize::from(fd)) {
            Some(Slot::File { node, at }) => (*at, node.size),
            Some(Slot::Out { at, len, .. }) => (*at as u64, *len as u64),
            _ => return Err(Error::BadHandle),
        };
        let from = match whence {
            Whence::Start => 0,
            Whence::Here => at,
            Whence::End => size,
        };
        let to = i64::try_from(from)
            .ok()
            .and_then(|f| f.checked_add(offset))
            .and_then(|t| u64::try_from(t).ok())
            .ok_or(Error::BadPath)?;
        match &mut self.slots[usize::from(fd)] {
            Slot::File { at, .. } => *at = to,
            Slot::Out { at, len, .. } => {
                if to > *len as u64 {
                    return Err(Error::BadPath);
                }
                *at = to as usize;
            }
            _ => {}
        }
        Ok(to)
    }

    /// Whether `fd` is the file being written.
    #[must_use]
    pub fn writing(&self, fd: Fd) -> bool {
        matches!(self.slots.get(usize::from(fd)), Some(Slot::Out { .. }))
    }

    /// The file's size (a written file's: what was written so far).
    ///
    /// # Errors
    /// Not a file's handle.
    pub fn size(&self, fd: Fd) -> Result<u64, Error> {
        match self.slots.get(usize::from(fd)) {
            Some(Slot::File { node, .. }) => Ok(node.size),
            Some(Slot::Out { len, .. }) => Ok(*len as u64),
            _ => Err(Error::BadHandle),
        }
    }

    /// Closes a handle. The written file goes onto the card now: its bytes
    /// first, out of sight, then its name (the old file of that name taken
    /// away just before); a failure leaves the old file where the card
    /// allows it and frees the handle.
    ///
    /// # Errors
    /// Not a handle; for the written file, no room on the card, or the
    /// card.
    pub fn close(&mut self, fd: Fd) -> Result<(), Error> {
        let slot = self
            .slots
            .get_mut(usize::from(fd))
            .ok_or(Error::BadHandle)?;
        match core::mem::replace(slot, Slot::Free) {
            Slot::Free => Err(Error::BadHandle),
            Slot::File { .. } | Slot::Dir { .. } => Ok(()),
            Slot::Out {
                dir,
                name,
                name_len,
                start,
                len,
                ..
            } => {
                let put = self.put_out(dir, &name[..name_len], start, len);
                // Its room goes back to the files after it, put or not.
                let end = self.out_end().max(start + len);
                self.room.copy_within(start + len..end, start);
                for slot in &mut self.slots {
                    if let Slot::Out { start: other, .. } = slot
                        && *other >= start + len
                    {
                        *other -= len;
                    }
                }
                put
            }
        }
    }

    /// Where the entry of `dir`'s child named `name` (any case) is, if
    /// there is one, read from the card.
    fn entry_named(&mut self, dir: u64, name: &[u16]) -> Result<Option<u64>, Error> {
        let mut node = Node::default();
        match self.walk(dir, name, &mut node) {
            Ok(()) => Ok(Some(node.loc)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Puts a written file's bytes (`len` of the room from `start`) onto
    /// the card under `name` in `dir`, whole or not at all.
    fn put_out(&mut self, dir: u64, name: &[u16], start: usize, len: usize) -> Result<(), Error> {
        // The file's folder changes, and the root for a moment (the
        // reservation's own entry).
        self.forget(dir);
        self.forget(ROOT);
        // An empty file takes no cluster, and the card's reservation is
        // for space (it refuses none): made as an entry alone, the old
        // one of that name gone first (found 2026-11-16: an empty file
        // written by a program failed to close with "no room").
        if len == 0 {
            if let Some(loc) = self.entry_named(dir, name)? {
                self.vol.node_delete(loc)?;
            }
            self.vol
                .node_create(dir, name, false, false, (None, None), self.scratch)?;
            return Ok(());
        }
        {
            {
                let mut r = self.vol.node_reserve(dir, len as u64, self.scratch)?;
                // In whole sectors (the node layer's rule), the last one
                // short.
                let step = (self.scratch.len() / 2).max(512) / 512 * 512;
                let mut at = 0;
                while at < len {
                    let n = step.min(len - at);
                    let piece = &self.room[start + at..start + at + n];
                    if let Err(e) = self.vol.node_write(&mut r, at as u64, piece, self.scratch) {
                        let _ = self.vol.node_abandon(&r);
                        return Err(e.into());
                    }
                    at += n;
                }
                // The old one of that name, if any, makes way.
                if let Some(loc) = self.entry_named(dir, name)? {
                    self.vol.node_delete(loc)?;
                }
                match self
                    .vol
                    .node_commit(&r, dir, name, false, (None, None), self.scratch)
                {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        let _ = self.vol.node_abandon(&r);
                        Err(e.into())
                    }
                }
            }
        }
    }

    /// Makes the folder at `path` (its parent must be there; one that is
    /// there already is no error, as a game makes its save folder each
    /// start).
    ///
    /// # Errors
    /// No such parent, a bad name, a file of that name, or the card.
    pub fn mkdir(&mut self, path: &[u8]) -> Result<(), Error> {
        let (dir, last) = self.parent(path)?;
        let mut node = Node::default();
        match self.child(dir, last, &mut node) {
            Ok(()) if node.folder => return Ok(()),
            Ok(()) => return Err(Error::BadPath),
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        let mut name = [0u16; NAME];
        let n = units(last, &mut name).ok_or(Error::BadPath)?;
        if !node::valid_name(&name[..n]) {
            return Err(Error::BadPath);
        }
        self.forget(dir);
        self.vol
            .node_create(dir, &name[..n], true, false, (None, None), self.scratch)?;
        Ok(())
    }

    /// Deletes the file (or empty folder) at `path`.
    ///
    /// # Errors
    /// No such file, or the card.
    pub fn remove(&mut self, path: &[u8]) -> Result<(), Error> {
        let mut node = Node::default();
        let (dir, name) = self.parent(path)?;
        if name.is_empty() || name == b"." {
            return Err(Error::BadPath);
        }
        self.child(dir, name, &mut node)?;
        if node.folder {
            self.forget_all();
        } else {
            self.forget(dir);
        }
        self.vol.node_delete(node.loc)?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rp_fat::mkfs::{self, Fs, Options};
    use rp_hal_api::BlockError;

    pub(crate) struct Disk(Vec<u8>);

    thread_local! {
        /// The room every table made in this thread gets for its
        /// folders' names (0: none), set by [`each`].
        pub(crate) static NAMES_ROOM: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        /// Read commands the test's cards have been sent.
        static READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    impl BlockDevice for Disk {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            READS.set(READS.get() + 1);
            let o = lba as usize * 512;
            if o + buf.len() > self.0.len() {
                return Err(BlockError::OutOfRange);
            }
            buf.copy_from_slice(&self.0[o..o + buf.len()]);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            if o + buf.len() > self.0.len() {
                return Err(BlockError::OutOfRange);
            }
            self.0[o..o + buf.len()].copy_from_slice(buf);
            Ok(())
        }
    }

    fn volume(fs: Fs, sectors: u32, cluster: u32) -> AnyVolume<Disk> {
        let o = Options {
            fs: Some(fs),
            cluster_bytes: Some(cluster),
            align_sectors: None,
            label: "GAME",
            serial: 7,
        };
        let l = mkfs::plan(sectors, &o).unwrap_or_else(|e| panic!("{}", e.name()));
        let mut d = Disk(vec![0; sectors as usize * 512]);
        mkfs::write(&mut d, &l, &mut vec![0u8; 16 * 1024]).unwrap();
        AnyVolume::mount(d, l.part_start).unwrap()
    }

    pub(crate) fn pattern(n: usize, seed: u8) -> Vec<u8> {
        (0..n)
            .map(|i| (i as u8).wrapping_mul(37) ^ seed ^ (i >> 8) as u8)
            .collect()
    }

    /// Every kind of card the X51v takes, each three times: as the
    /// card alone answers, with the folders' names remembered, and with
    /// room for only a few of them (some folders remembered, the rest
    /// walked).
    pub(crate) fn each(mut f: impl FnMut(AnyVolume<Disk>, &str)) {
        for (room, how) in [
            (0, ""),
            (64 * 1024, ", names kept"),
            (150, ", names short of room"),
        ] {
            NAMES_ROOM.set(room);
            f(volume(Fs::Fat32, 131_072, 512), &format!("FAT32{how}"));
            f(volume(Fs::Fat16, 65_536, 2048), &format!("FAT16{how}"));
            f(volume(Fs::Fat12, 16_384, 4096), &format!("FAT12{how}"));
            f(volume(Fs::ExFat, 65_536, 4096), &format!("exFAT{how}"));
        }
        NAMES_ROOM.set(0);
    }

    /// Every name `fd` lists from here to its end.
    fn names_listed(f: &mut Files<'_, Disk, 8>, fd: Fd, most: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut name = [0u8; 255];
        while out.len() < most {
            match f.next(fd, &mut name).unwrap() {
                Some(e) => out.push(String::from_utf8_lossy(&name[..e.name_len]).into_owned()),
                None => break,
            }
        }
        out
    }

    #[test]
    fn a_folder_listed_again_costs_no_card_read_and_a_change_while_listing_loses_nothing() {
        // No names kept; kept; and room to list the folder once but not
        // again after it changes (the walk that keeps the names' order).
        for (room, how) in [(0, "card"), (64 * 1024, "names"), (2000, "names, then full")] {
            for (fs, cluster) in [(Fs::Fat32, 512), (Fs::ExFat, 4096)] {
                NAMES_ROOM.set(room);
                let vol = volume(fs, 131_072, cluster);
                let (scratch, files) = (&mut vec![0u8; 8192][..], &mut vec![0u8; 100_000][..]);
                let mut f: Files<'_, Disk, 8> = Files::new(vol, scratch, files);
                NAMES_ROOM.set(0);
                f.mkdir(b"maps").unwrap();
                let made: Vec<String> = (0..30).map(|i| format!("file-number-{i:02}.bsp")).collect();
                for n in &made {
                    let w = f.create(format!("maps/{n}").as_bytes()).unwrap();
                    f.close(w).unwrap_or_else(|e| panic!("{how} {fs:?} {n}: {e:?}"));
                }
                // Another folder kept: its names hold the room's bytes when
                // `maps` is forgotten, so `maps` may not fit again.
                f.mkdir(b"sound").unwrap();
                for n in ["s0.wav", "s1.wav", "s2.wav"] {
                    let w = f.create(format!("sound/{n}").as_bytes()).unwrap();
                    f.write(w, b"x").unwrap();
                    f.close(w).unwrap();
                }
                let d = f.open_dir(b"maps").unwrap();
                assert_eq!(f.stat(b"sound/s1.wav").unwrap().size, 1);
                let mut first = names_listed(&mut f, d, usize::MAX);
                f.close(d).unwrap();
                first.sort();
                assert_eq!(first, made, "{how} {fs:?}");
                // Listed again: from the names, no card read.
                let before = READS.get();
                let d = f.open_dir(b"/MAPS").unwrap();
                let again = names_listed(&mut f, d, usize::MAX);
                f.close(d).unwrap();
                if room == 64 * 1024 {
                    assert_eq!(READS.get() - before, 0, "{fs:?}");
                }
                assert_eq!(again.len(), 30, "{how} {fs:?}");
                // Changed part way: a name before where the listing is, one
                // after it, and one removed that was not listed yet.
                let d = f.open_dir(b"maps").unwrap();
                let mut seen = names_listed(&mut f, d, 10);
                for n in ["aaa-new.bsp", "zzz-new.bsp"] {
                    let w = f.create(format!("maps/{n}").as_bytes()).unwrap();
                    f.close(w).unwrap();
                }
                let gone = made
                    .iter()
                    .find(|n| !seen.contains(n))
                    .unwrap()
                    .clone();
                f.remove(format!("maps/{gone}").as_bytes()).unwrap();
                seen.extend(names_listed(&mut f, d, usize::MAX));
                f.close(d).unwrap();
                for n in &made {
                    let times = seen.iter().filter(|s| *s == n).count();
                    let ok = if *n == gone { times <= 1 } else { times == 1 };
                    assert!(ok, "{how} {fs:?}: {n} listed {times} times in {seen:?}");
                }
                let mut once = seen.clone();
                once.sort();
                once.dedup();
                assert_eq!(once.len(), seen.len(), "{how} {fs:?}: a name twice in {seen:?}");
                if room != 0 {
                    // In the names' order a name after the place is listed.
                    assert!(seen.iter().any(|s| s == "zzz-new.bsp"), "{how} {fs:?}");
                }
                if room == 2000 {
                    // The folder changed with the room too full to list it
                    // again: the walk in the names' order answered.
                    assert!(f.remembered().unwrap().refused > 0, "{fs:?}");
                }
                // An empty file written over one with bytes: there, empty.
                let w = f.create(b"maps/aaa-new.bsp").unwrap();
                f.write(w, b"bytes").unwrap();
                f.close(w).unwrap();
                assert_eq!(f.stat(b"maps/aaa-new.bsp").unwrap().size, 5);
                let w = f.create(b"maps/AAA-NEW.bsp").unwrap();
                f.close(w).unwrap();
                assert_eq!(f.stat(b"maps/aaa-new.bsp").unwrap().size, 0, "{how} {fs:?}");
                let r = f.open(b"maps/aaa-new.bsp").unwrap();
                assert_eq!(f.read(r, &mut [0u8; 8]), Ok(0));
                f.close(r).unwrap();
            }
        }
    }

    #[test]
    fn a_path_asked_about_again_costs_no_card_read() {
        for (fs, cluster) in [(Fs::Fat32, 512), (Fs::ExFat, 4096)] {
            NAMES_ROOM.set(0);
            let vol = volume(fs, 131_072, cluster);
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 100_000][..];
            let names = &mut vec![0u8; 32 * 1024][..];
            let mut f: Files<'_, Disk, 8> = Files::new(vol, scratch, room);
            f.mkdir(b"Ports").unwrap();
            f.mkdir(b"Ports/valve").unwrap();
            for i in 0..40 {
                let w = f
                    .create(format!("Ports/valve/file-number-{i}.txt").as_bytes())
                    .unwrap();
                f.write(w, &pattern(100 + i, 1)).unwrap();
                f.close(w).unwrap();
            }
            // After the files, as a card holds a folder made later.
            f.mkdir(b"Ports/valve/gfx").unwrap();
            f.mkdir(b"Ports/valve/gfx/shell").unwrap();
            let cost = |f: &mut Files<'_, Disk, 8>, path: &[u8]| {
                let before = READS.get();
                let got = f.stat(path);
                (got, READS.get() - before)
            };
            // As it was: every ask walks every folder on the way.
            let (plain, walked) = cost(&mut f, b"/Ports/valve/gfx/shell");
            assert!(walked > 40, "{walked} reads without the names");
            let (gone, _) = cost(&mut f, b"/Ports/valve/gfx/shell/btns_main.bmp");
            assert_eq!(gone, Err(Error::NotFound));
            // With the names: the first ask lists the folders, the ones
            // after read nothing, there or not, and say the same.
            f.remember(Names::new(names));
            assert_eq!(cost(&mut f, b"/Ports/valve/gfx/shell").0, plain);
            for path in [
                &b"/Ports/valve/gfx/shell"[..],
                b"/ports/VALVE/gfx",
                b"/Ports/valve/file-number-39.txt",
            ] {
                let (got, reads) = cost(&mut f, path);
                assert!(got.is_ok());
                assert_eq!(reads, 0, "{}", String::from_utf8_lossy(path));
            }
            assert_eq!(
                cost(&mut f, b"/Ports/valve/file-number-39.txt")
                    .0
                    .unwrap()
                    .size,
                139
            );
            // The folder's first ask lists it (empty); then nothing.
            assert_eq!(
                cost(&mut f, b"/Ports/valve/gfx/shell/btns_main.bmp").0,
                gone
            );
            for path in [
                &b"/Ports/valve/gfx/shell/btns_main.bmp"[..],
                b"/Ports/valve/gfx/shell/x/y.bmp",
                b"/Ports/valve/nothing.wad",
                b"/Ports/nowhere/gfx.wad",
            ] {
                let (got, reads) = cost(&mut f, path);
                assert_eq!(got, Err(Error::NotFound));
                assert_eq!(reads, 0, "{}", String::from_utf8_lossy(path));
            }
            // A file that is there opens with its entry read once.
            let before = READS.get();
            let r = f.open(b"/Ports/valve/file-number-7.txt").unwrap();
            assert!(READS.get() - before <= 2, "{} reads", READS.get() - before);
            let mut got = [0u8; 200];
            assert_eq!(f.read(r, &mut got), Ok(107));
            assert_eq!(&got[..107], &pattern(107, 1)[..]);
            f.close(r).unwrap();
            // What changes is seen at once: a new file, a file gone, a
            // folder renamed with all under it.
            let w = f.create(b"/Ports/valve/gfx/shell/btns_main.bmp").unwrap();
            f.write(w, b"bmp").unwrap();
            f.close(w).unwrap();
            assert_eq!(
                f.stat(b"/Ports/valve/gfx/shell/btns_main.bmp")
                    .unwrap()
                    .size,
                3
            );
            f.remove(b"/Ports/valve/file-number-7.txt").unwrap();
            assert_eq!(
                f.stat(b"/Ports/valve/file-number-7.txt"),
                Err(Error::NotFound)
            );
            f.rename(b"/Ports/valve/gfx", b"/Ports/valve/gfx2").unwrap();
            assert_eq!(f.stat(b"/Ports/valve/gfx/shell"), Err(Error::NotFound));
            assert_eq!(
                f.stat(b"/Ports/valve/gfx2/shell/btns_main.bmp")
                    .unwrap()
                    .size,
                3
            );
            assert_eq!(cost(&mut f, b"/Ports/valve/gfx2/shell/btns_main.bmp").1, 0);
            let used = f.remembered().unwrap();
            assert!(used.folders >= 4 && used.refused == 0, "{used:?}");
        }
    }

    #[test]
    fn a_game_s_files_are_made_found_read_and_listed() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 300_000][..];
            let mut f: Files<'_, Disk, 8> = Files::new(vol, scratch, room);
            // Folders as a game has them; making one twice is no error.
            f.mkdir(b"valve").unwrap();
            f.mkdir(b"valve/maps").unwrap();
            f.mkdir(b"valve/maps").unwrap();
            assert_eq!(f.mkdir(b"nowhere/maps"), Err(Error::NotFound), "{kind}");
            // A map written, of a size that ends mid-sector.
            let map = pattern(200_123, 3);
            let w = f.create(b"valve/maps/c1a0.bsp").unwrap();
            for piece in map.chunks(7_000) {
                assert_eq!(f.write(w, piece), Ok(piece.len()));
            }
            assert_eq!(f.size(w), Ok(map.len() as u64));
            // Not on the card until it is closed.
            assert_eq!(
                f.open(b"valve/maps/c1a0.bsp"),
                Err(Error::NotFound),
                "{kind}"
            );
            f.close(w).unwrap();
            // Found whatever the case or the slashes; read in pieces.
            let r = f.open(b"\\VALVE\\Maps\\C1A0.BSP").unwrap();
            assert_eq!(f.size(r), Ok(map.len() as u64));
            let mut got = vec![0u8; map.len() + 50];
            let mut n = 0;
            loop {
                let k = f
                    .read(r, &mut got[n..n + 4_099.min(map.len() + 50 - n)])
                    .unwrap();
                if k == 0 {
                    break;
                }
                n += k;
            }
            assert_eq!(n, map.len(), "{kind}");
            assert_eq!(&got[..n], &map[..], "{kind}");
            // Seeks: from the start, from here, from the end; past the end
            // reads nothing; before the start is refused.
            assert_eq!(f.seek(r, 100_000, Whence::Start), Ok(100_000));
            let mut four = [0u8; 4];
            f.read(r, &mut four).unwrap();
            assert_eq!(four, map[100_000..100_004]);
            assert_eq!(f.seek(r, -4, Whence::Here), Ok(100_000));
            assert_eq!(f.seek(r, -3, Whence::End), Ok(map.len() as u64 - 3));
            assert_eq!(f.read(r, &mut four), Ok(3));
            assert_eq!(f.read(r, &mut four), Ok(0));
            assert_eq!(f.seek(r, 10, Whence::End), Ok(map.len() as u64 + 10));
            assert_eq!(f.read(r, &mut four), Ok(0));
            assert_eq!(f.seek(r, -1, Whence::Start), Err(Error::BadPath));
            f.close(r).unwrap();
            assert_eq!(f.close(r), Err(Error::BadHandle));
            assert_eq!(f.read(r, &mut four), Err(Error::BadHandle));
            // A folder listed: its entries, each once.
            let small = f.create(b"valve/liblist.gam").unwrap();
            f.write(small, b"game \"Half-Life\"\n").unwrap();
            f.close(small).unwrap();
            let d = f.open_dir(b"valve").unwrap();
            let mut seen = Vec::new();
            let mut name = [0u8; 64];
            while let Some(e) = f.next(d, &mut name).unwrap() {
                seen.push((
                    String::from_utf8_lossy(&name[..e.name_len]).to_lowercase(),
                    e.folder,
                    e.size,
                ));
            }
            seen.sort();
            assert_eq!(
                seen,
                [
                    ("liblist.gam".to_string(), false, 17),
                    ("maps".to_string(), true, 0)
                ],
                "{kind}"
            );
            assert_eq!(
                f.read(d, &mut four),
                Err(Error::BadHandle),
                "a folder is not read"
            );
            f.close(d).unwrap();
            // The root lists too.
            let d = f.open_dir(b"/").unwrap();
            assert!(f.next(d, &mut name).unwrap().is_some_and(|e| e.folder));
            f.close(d).unwrap();
            // A file is not a folder, and `..` is refused.
            assert_eq!(f.open(b"valve/liblist.gam/x"), Err(Error::BadPath));
            assert_eq!(f.open(b"valve/../valve/liblist.gam"), Err(Error::BadPath));
            assert_eq!(
                f.open(b"valve"),
                Err(Error::BadPath),
                "a folder is not opened as a file"
            );
        });
    }

    #[test]
    fn several_files_are_written_at_once_and_each_reaches_the_card_whole() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0xEEu8; 10_000][..];
            let mut f: Files<'_, Disk, 6> = Files::new(vol, scratch, room);
            let whole = |f: &mut Files<'_, Disk, 6>, path: &[u8]| {
                let r = f.open(path).unwrap();
                let mut got = vec![0u8; f.size(r).unwrap() as usize];
                let mut n = 0;
                while n < got.len() {
                    n += f.read(r, &mut got[n..]).unwrap();
                }
                f.close(r).unwrap();
                got
            };
            let (a_bytes, b_bytes, c_bytes) =
                (pattern(3_000, 1), pattern(2_500, 2), pattern(700, 3));
            // A recording, a save and a log, opened one after another and
            // written in turns, the first growing after the others exist.
            let a = f.create(b"demo.dem").unwrap();
            f.write(a, &a_bytes[..100]).unwrap();
            let b = f.create(b"save.sav").unwrap();
            let c = f.create(b"log.txt").unwrap();
            for step in 0..10 {
                let part = |bytes: &[u8], from: usize| {
                    let n = (bytes.len() - from) / (10 - step);
                    (from, from + n)
                };
                let (x, y) = part(&b_bytes, f.size(b).unwrap() as usize);
                assert_eq!(f.write(b, &b_bytes[x..y]), Ok(y - x), "{kind} step {step}");
                let (x, y) = part(&a_bytes, f.size(a).unwrap() as usize);
                assert_eq!(f.write(a, &a_bytes[x..y]), Ok(y - x));
                let (x, y) = part(&c_bytes, f.size(c).unwrap() as usize);
                assert_eq!(f.write(c, &c_bytes[x..y]), Ok(y - x));
            }
            assert_eq!(
                (f.size(a), f.size(b), f.size(c)),
                (Ok(3_000), Ok(2_500), Ok(700))
            );
            // Each reads back as written before it is closed.
            f.seek(b, 0, Whence::Start).unwrap();
            let mut back = vec![0u8; 2_500];
            assert_eq!(f.read(b, &mut back), Ok(2_500));
            assert_eq!(back, b_bytes, "{kind}");
            // The room is what they share: 6,200 of 10,000 are taken, so
            // 3,800 more fit in any of them and 3,801 in none (nothing
            // taken, nothing moved).
            let fill = vec![7u8; 3_801];
            assert_eq!(f.write(c, &fill), Err(Error::NoRoom));
            assert_eq!(f.write(a, &fill), Err(Error::NoRoom));
            assert_eq!(f.size(c), Ok(700));
            // The middle one closed: on the card whole, and its room is
            // the others' again.
            f.close(b).unwrap();
            assert_eq!(whole(&mut f, b"save.sav"), b_bytes, "{kind}");
            assert_eq!(f.write(c, &fill[..3_800]), Ok(3_800));
            let mut c_all = c_bytes.clone();
            c_all.extend(&fill[..3_800]);
            // One taken in to be added to while the others are open, and
            // the first rewritten at its start (it does not grow).
            f.seek(a, 0, Whence::Start).unwrap();
            f.write(a, b"HEAD").unwrap();
            let mut a_all = a_bytes.clone();
            a_all[..4].copy_from_slice(b"HEAD");
            let d = f.reopen(b"save.sav", true, false).unwrap();
            assert_eq!(f.write(d, b"more"), Err(Error::NoRoom), "the room is full");
            f.close(a).unwrap();
            assert_eq!(f.write(d, b"more"), Ok(4));
            f.close(d).unwrap();
            f.close(c).unwrap();
            assert_eq!(whole(&mut f, b"demo.dem"), a_all, "{kind}");
            assert_eq!(whole(&mut f, b"log.txt"), c_all, "{kind}");
            let mut b_all = b_bytes.clone();
            b_all.extend(b"more");
            assert_eq!(whole(&mut f, b"save.sav"), b_all, "{kind}");
            // Nothing is left in the room: a file of its whole size fits.
            let big = pattern(10_000, 9);
            let w = f.create(b"big.bin").unwrap();
            assert_eq!(f.write(w, &big), Ok(10_000));
            f.close(w).unwrap();
            assert_eq!(whole(&mut f, b"big.bin"), big, "{kind}");
        });
    }

    #[test]
    fn a_log_is_appended_to_and_a_file_changed_in_place() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 4096][..];
            let room = &mut vec![0u8; 1_000][..];
            let mut f: Files<'_, Disk, 4> = Files::new(vol, scratch, room);
            // Appending to nothing: refused unless it may be made.
            assert_eq!(
                f.reopen(b"log.txt", true, false),
                Err(Error::NotFound),
                "{kind}"
            );
            let w = f.reopen(b"log.txt", true, true).unwrap();
            f.write(w, b"first\n").unwrap();
            f.close(w).unwrap();
            // Appended: what was there stays, the new goes after it.
            let w = f.reopen(b"LOG.TXT", true, false).unwrap();
            assert_eq!(f.size(w), Ok(6));
            f.write(w, b"second\n").unwrap();
            f.close(w).unwrap();
            // Changed in place from its start ("r+"): the rest stays.
            let w = f.reopen(b"log.txt", false, false).unwrap();
            f.write(w, b"FIRST").unwrap();
            f.close(w).unwrap();
            let r = f.open(b"log.txt").unwrap();
            let mut got = [0u8; 32];
            let n = f.read(r, &mut got).unwrap();
            assert_eq!(&got[..n], b"FIRST\nsecond\n", "{kind}");
            f.close(r).unwrap();
            // A file bigger than the room cannot be taken in: said, and
            // the handle is free again; a folder is no file.
            let w = f.create(b"big.bin").unwrap();
            f.write(w, &[7u8; 1_000]).unwrap();
            f.close(w).unwrap();
            let small = &mut vec![0u8; 999][..];
            let scratch2 = &mut vec![0u8; 4096][..];
            let mut g: Files<'_, Disk, 1> = Files::new(f.into_volume(), scratch2, small);
            assert_eq!(
                g.reopen(b"big.bin", true, false),
                Err(Error::NoRoom),
                "{kind}"
            );
            g.mkdir(b"dir").unwrap();
            assert_eq!(g.reopen(b"dir", false, false), Err(Error::BadPath));
            assert_eq!(g.reopen(b"nowhere/x", true, true), Err(Error::NotFound));
            let r = g.open(b"big.bin").unwrap();
            g.close(r).unwrap();
            let mut vol = g.into_volume();
            let health = vol
                .check(&mut vec![0u8; 16_384], &mut vec![0u8; 8192])
                .unwrap();
            assert!(health.clean(), "{kind}: {health:?}");
        });
    }

    #[test]
    fn a_second_handle_has_its_own_place_and_a_path_says_what_it_names() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 4096][..];
            let room = &mut vec![0u8; 5_000][..];
            let mut f: Files<'_, Disk, 3> = Files::new(vol, scratch, room);
            f.mkdir(b"valve").unwrap();
            let data = pattern(3_000, 5);
            let w = f.create(b"valve/pak0.pak").unwrap();
            f.write(w, &data).unwrap();
            // The file being written has one handle only.
            assert_eq!(f.dup(w), Err(Error::BadHandle));
            f.close(w).unwrap();
            // A pack file read through two handles at two places.
            let a = f.open(b"valve/pak0.pak").unwrap();
            f.seek(a, 1_000, Whence::Start).unwrap();
            let b = f.dup(a).unwrap();
            assert_ne!(a, b);
            f.seek(b, 2_000, Whence::Start).unwrap();
            let (mut x, mut y) = ([0u8; 100], [0u8; 100]);
            assert_eq!(f.read(a, &mut x), Ok(100));
            assert_eq!(f.read(b, &mut y), Ok(100));
            assert_eq!(
                (&x[..], &y[..]),
                (&data[1_000..1_100], &data[2_000..2_100]),
                "{kind}"
            );
            // No handle left for a third; not a handle at all.
            let c = f.open_dir(b"valve").unwrap();
            assert_eq!(f.dup(a), Err(Error::NoHandle));
            f.close(c).unwrap();
            assert_eq!(f.dup(c), Err(Error::BadHandle));
            assert_eq!(f.dup(99), Err(Error::BadHandle));
            f.close(a).unwrap();
            // Closing one leaves the other reading.
            assert_eq!(f.read(b, &mut y), Ok(100));
            assert_eq!(&y[..], &data[2_100..2_200]);
            f.close(b).unwrap();
            // What a path names.
            let s = f.stat(b"VALVE/PAK0.PAK").unwrap();
            assert_eq!((s.folder, s.size), (false, 3_000), "{kind}");
            let s = f.stat(b"valve").unwrap();
            assert_eq!((s.folder, s.size), (true, 0));
            for root in [&b""[..], b"/", b"\\"] {
                assert!(f.stat(root).unwrap().folder);
            }
            assert_eq!(f.stat(b"valve/none.bsp"), Err(Error::NotFound));
            assert_eq!(f.stat(b"valve/pak0.pak/x"), Err(Error::BadPath));
        });
    }

    #[test]
    fn a_file_is_renamed_moved_and_takes_another_s_place() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 8192][..];
            let room = &mut vec![0u8; 100][..];
            let mut f: Files<'_, Disk, 2> = Files::new(vol, scratch, room);
            let put = |f: &mut Files<'_, Disk, 2>, path: &[u8], text: &[u8]| {
                let w = f.create(path).unwrap();
                f.write(w, text).unwrap();
                f.close(w).unwrap();
            };
            let get = |f: &mut Files<'_, Disk, 2>, path: &[u8]| -> Vec<u8> {
                let r = f.open(path).unwrap();
                let mut b = [0u8; 100];
                let n = f.read(r, &mut b).unwrap();
                f.close(r).unwrap();
                b[..n].to_vec()
            };
            f.mkdir(b"save").unwrap();
            f.mkdir(b"old").unwrap();
            put(&mut f, b"save/quick.tmp", b"new save");
            put(&mut f, b"save/quick.sav", b"old save");
            // A new name in the same folder; the old name is gone.
            f.rename(b"save/quick.tmp", b"save/second.sav").unwrap();
            assert_eq!(get(&mut f, b"save/second.sav"), b"new save", "{kind}");
            assert_eq!(f.stat(b"save/quick.tmp"), Err(Error::NotFound));
            // Onto a file that is there: it makes way (a save written
            // beside the old one, then given its name).
            f.rename(b"save/second.sav", b"save/quick.sav").unwrap();
            assert_eq!(get(&mut f, b"save/quick.sav"), b"new save", "{kind}");
            assert_eq!(f.stat(b"save/second.sav"), Err(Error::NotFound));
            // Into another folder, under another name and under its own.
            f.rename(b"save/quick.sav", b"old/kept.sav").unwrap();
            assert_eq!(get(&mut f, b"old/kept.sav"), b"new save", "{kind}");
            f.rename(b"old/kept.sav", b"save/kept.sav").unwrap();
            assert_eq!(get(&mut f, b"save/kept.sav"), b"new save", "{kind}");
            // Its own name in another case is no clash with itself.
            f.rename(b"save/kept.sav", b"save/KEPT.SAV").unwrap();
            assert_eq!(get(&mut f, b"save/kept.sav"), b"new save");
            // A folder is renamed too; a file does not take a folder's
            // place; nothing, nowhere and no name are refused.
            f.rename(b"old", b"older").unwrap();
            assert!(f.stat(b"older").unwrap().folder);
            assert_eq!(
                f.rename(b"save/kept.sav", b"older"),
                Err(Error::BadPath),
                "{kind}"
            );
            assert_eq!(f.rename(b"save/none", b"save/x"), Err(Error::NotFound));
            assert_eq!(
                f.rename(b"save/kept.sav", b"nowhere/x"),
                Err(Error::NotFound)
            );
            assert_eq!(f.rename(b"save/kept.sav", b"save/"), Err(Error::BadPath));
            assert_eq!(get(&mut f, b"save/kept.sav"), b"new save");
            let mut vol = f.into_volume();
            let health = vol
                .check(&mut vec![0u8; 16_384], &mut vec![0u8; 8192])
                .unwrap();
            assert!(health.clean(), "{kind}: {health:?}");
        });
    }

    #[test]
    fn a_card_s_date_is_seconds_since_1970() {
        let at = |year, month, day, hour, minute, second| {
            unix_seconds(&Stamp {
                year,
                month,
                day,
                hour,
                minute,
                second,
                centis: None,
                zone: None,
            })
        };
        assert_eq!(at(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(at(1980, 1, 1, 0, 0, 0), 315_532_800, "FAT's first day");
        assert_eq!(at(2000, 2, 29, 12, 0, 0), 951_825_600, "a leap day");
        assert_eq!(at(2000, 3, 1, 0, 0, 0), 951_868_800);
        assert_eq!(at(2026, 11, 15, 23, 59, 59), 1_794_787_199);
        assert_eq!(
            at(2100, 3, 1, 0, 0, 0),
            4_107_542_400,
            "2100 is no leap year"
        );
        assert_eq!(at(2107, 12, 31, 23, 59, 58), 4_354_819_198, "FAT's last");
        // No date at all (an entry never stamped), or one before 1970.
        assert_eq!(at(0, 0, 0, 0, 0, 0), 0);
        assert_eq!(at(1969, 12, 31, 23, 59, 59), 0);
        assert_eq!(at(2020, 13, 1, 0, 0, 0), 0);
    }

    #[test]
    fn a_save_replaces_the_old_one_whole_and_handles_run_out_cleanly() {
        each(|vol, kind| {
            let scratch = &mut vec![0u8; 4096][..];
            let room = &mut vec![0u8; 40_000][..];
            let mut f: Files<'_, Disk, 3> = Files::new(vol, scratch, room);
            f.mkdir(b"save").unwrap();
            for round in 0..3u8 {
                let data = pattern(10_000 + usize::from(round) * 4_321, round);
                let w = f.create(b"save/quick.sav").unwrap();
                f.write(w, &data).unwrap();
                // What was written reads back before it is closed.
                assert_eq!(f.seek(w, 0, Whence::Start), Ok(0));
                let mut head = [0u8; 16];
                assert_eq!(f.read(w, &mut head), Ok(16));
                assert_eq!(head, data[..16]);
                f.close(w).unwrap();
                let r = f.open(b"save/quick.sav").unwrap();
                let mut got = vec![0u8; data.len()];
                assert_eq!(f.read(r, &mut got), Ok(data.len()), "{kind} round {round}");
                assert_eq!(got, data, "{kind} round {round}");
                f.close(r).unwrap();
            }
            // One file of that name in the folder, not three.
            let d = f.open_dir(b"save").unwrap();
            let mut name = [0u8; 32];
            let mut n = 0;
            while f.next(d, &mut name).unwrap().is_some() {
                n += 1;
            }
            assert_eq!(n, 1, "{kind}");
            f.close(d).unwrap();
            // More than the room: refused, nothing taken; the file as far
            // as it got still closes onto the card.
            let w = f.create(b"save/big.sav").unwrap();
            assert_eq!(f.write(w, &vec![1u8; 39_000]), Ok(39_000));
            assert_eq!(f.write(w, &vec![2u8; 2_000]), Err(Error::NoRoom));
            f.close(w).unwrap();
            // Every handle taken: said, and one closed makes room.
            let a = f.open(b"save/quick.sav").unwrap();
            let b = f.open(b"save/big.sav").unwrap();
            let c = f.open_dir(b"save").unwrap();
            assert_eq!(f.open(b"save/quick.sav"), Err(Error::NoHandle));
            f.close(b).unwrap();
            let b = f.open(b"save/big.sav").unwrap();
            assert_eq!(f.size(b), Ok(39_000));
            for fd in [a, b, c] {
                f.close(fd).unwrap();
            }
            // Removed is gone.
            f.remove(b"save/big.sav").unwrap();
            assert_eq!(f.open(b"save/big.sav"), Err(Error::NotFound), "{kind}");
            assert_eq!(f.remove(b"save/big.sav"), Err(Error::NotFound));
            // The card's own check finds it sound after all of it.
            let mut vol = f.into_volume();
            let health = vol
                .check(&mut vec![0u8; 16_384], &mut vec![0u8; 8192])
                .unwrap();
            assert!(health.clean(), "{kind}: {health:?}");
        });
    }
}
