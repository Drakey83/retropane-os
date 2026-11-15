//! Files as a C program uses them: open by path, read, seek, write, list a
//! folder, over the card's volume.

use rp_fat::node::{self, Node, ROOT};
use rp_fat::{AnyVolume, Error as FatError};
use rp_hal_api::BlockDevice;

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
    /// A folder being listed.
    Dir {
        dir: u64,
        cursor: u64,
    },
    /// The file being written: its folder, its name, how far the writing
    /// has got (its bytes are in the table's room).
    Out {
        dir: u64,
        name: [u16; NAME],
        name_len: usize,
        at: usize,
        len: usize,
    },
}

/// The table of open files over one volume: `N` handles, `scratch` for the
/// file system's sector work (the bigger, the longer its reads in one
/// command), `room` for the one file being written.
pub struct Files<'a, D: BlockDevice, const N: usize> {
    vol: AnyVolume<D>,
    slots: [Slot; N],
    scratch: &'a mut [u8],
    room: &'a mut [u8],
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
        }
    }

    /// The volume back (the card handed on).
    pub fn into_volume(self) -> AnyVolume<D> {
        self.vol
    }

    /// The child of folder `dir` named `name`, any case.
    fn child(&mut self, dir: u64, name: &[u8], out: &mut Node) -> Result<(), Error> {
        let mut want = [0u16; node::NAME_UNITS];
        let n = units(name, &mut want).ok_or(Error::BadPath)?;
        let mut cursor = 0;
        while self.vol.node_child(dir, &mut cursor, out)? {
            if node::same_name(out.name(), &want[..n]) {
                return Ok(());
            }
        }
        Err(Error::NotFound)
    }

    /// The folder a path's last name lies in, and that name: every name
    /// before it walked from the root (`.` skipped; `..` is not a name a
    /// game's files use, and is refused).
    fn parent<'p>(&mut self, path: &'p [u8]) -> Result<(u64, &'p [u8]), Error> {
        let mut dir = ROOT;
        let (mut name, mut rest) = split(path);
        let mut node = Node::default();
        loop {
            if name == b".." {
                return Err(Error::BadPath);
            }
            let (next, after) = split(rest);
            if next.is_empty() {
                return Ok((dir, name));
            }
            if name != b"." {
                self.child(dir, name, &mut node)?;
                if !node.folder {
                    return Err(Error::BadPath);
                }
                dir = node.loc;
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
        if self.slots.iter().any(|s| matches!(s, Slot::Out { .. })) {
            return Err(Error::NoRoom);
        }
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
            at: 0,
            len: 0,
        };
        Ok(k as Fd)
    }

    /// Opens the folder at `path` for listing (`""` or `"/"`: the root).
    ///
    /// # Errors
    /// No such folder, no handle left, or the card.
    pub fn open_dir(&mut self, path: &[u8]) -> Result<Fd, Error> {
        let k = self.free_slot()?;
        let dir = if split(path).0.is_empty() {
            ROOT
        } else {
            let mut node = Node::default();
            self.find(path, &mut node)?;
            if !node.folder {
                return Err(Error::BadPath);
            }
            node.loc
        };
        self.slots[k] = Slot::Dir { dir, cursor: 0 };
        Ok(k as Fd)
    }

    /// The folder's next entry, its name into `name` (cut to its length;
    /// units above a byte as `?`); `None` at its end.
    ///
    /// # Errors
    /// Not a folder's handle, or the card.
    pub fn next(&mut self, fd: Fd, name: &mut [u8]) -> Result<Option<Entry>, Error> {
        let Some(Slot::Dir { dir, cursor }) = self.slots.get_mut(usize::from(fd)) else {
            return Err(Error::BadHandle);
        };
        let mut node = Node::default();
        if !self.vol.node_child(*dir, cursor, &mut node)? {
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
            Some(Slot::Out { at, len, .. }) => {
                let n = buf.len().min(*len - *at);
                buf[..n].copy_from_slice(&self.room[*at..*at + n]);
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
        let Some(Slot::Out { at, len, .. }) = self.slots.get_mut(usize::from(fd)) else {
            return Err(Error::BadHandle);
        };
        let end = at.checked_add(data.len()).ok_or(Error::NoRoom)?;
        if end > self.room.len() {
            return Err(Error::NoRoom);
        }
        self.room[*at..end].copy_from_slice(data);
        *at = end;
        *len = (*len).max(end);
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
                len,
                ..
            } => {
                let name = &name[..name_len];
                let mut r = self.vol.node_reserve(dir, len as u64, self.scratch)?;
                // In whole sectors (the node layer's rule), the last one
                // short.
                let step = (self.scratch.len() / 2).max(512) / 512 * 512;
                let mut at = 0;
                while at < len {
                    let n = step.min(len - at);
                    let piece = &self.room[at..at + n];
                    if let Err(e) = self.vol.node_write(&mut r, at as u64, piece, self.scratch) {
                        let _ = self.vol.node_abandon(&r);
                        return Err(e.into());
                    }
                    at += n;
                }
                // The old one of that name, if any, makes way.
                let mut old = Node::default();
                let mut cursor = 0;
                let mut had = None;
                while self.vol.node_child(dir, &mut cursor, &mut old)? {
                    if node::same_name(old.name(), name) {
                        had = Some(old.loc);
                        break;
                    }
                }
                if let Some(loc) = had {
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
        self.find(path, &mut node)?;
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

    impl BlockDevice for Disk {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
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

    /// Every kind of card the X51v takes.
    pub(crate) fn each(mut f: impl FnMut(AnyVolume<Disk>, &str)) {
        f(volume(Fs::Fat32, 131_072, 512), "FAT32");
        f(volume(Fs::Fat16, 65_536, 2048), "FAT16");
        f(volume(Fs::Fat12, 16_384, 4096), "FAT12");
        f(volume(Fs::ExFat, 65_536, 4096), "exFAT");
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
            assert_eq!(
                f.create(b"valve/other.bin"),
                Err(Error::NoRoom),
                "one at a time"
            );
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
