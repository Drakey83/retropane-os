//! Modules: the libraries of a big program (a game port), as files placed
//! at build time (ADR-0039 rules 2, 3 and 4). This file is the format, read
//! and written the same way by the build (`xtask/src/module.rs`, which
//! packs a module) and by the runner, and the runner's loader
//! ([`Modules`]): safe Rust, tested on the PC with damaged, cut-short and
//! mismatched files.
//!
//! Iterate, don't imitate (rule 0). Linux and the consoles' ports load an
//! ELF shared object: position-independent code, a symbol hash table, and
//! relocations walked at every start. Windows loads a PE and patches it.
//! Ours is what is left when the OS and the game are built by one tool on
//! one day: the module is linked at the address it runs at against the
//! runner's own addresses, so the file is the memory image behind one
//! 72-byte header, and loading it is a read, a check and a zeroing. The
//! header says which runner build it was linked against; a module for
//! another runner is refused, never run. Names are two tables the build
//! sorts (by name; by address), each a binary search, so the engine's
//! look-up of every entity class and a saved game's name for a function
//! cost a dozen comparisons with nothing built at load.
//!
//! The check sum is XXH32, not the CRC-32 the loader's serial frames use
//! (`rp-loadproto`: one table look-up a byte): four multiply-and-rotate
//! lanes over 16 bytes at a time, on a CPU with a fast multiplier and no
//! CRC instruction. It guards against a damaged or cut-short file, not
//! against an attacker (ADR-0039 rule 10). Its speed on the X51v is not
//! measured yet (docs/reminders.md).

/// A module file's first four bytes.
pub const MAGIC: [u8; 4] = *b"RPMD";
/// The format's version: a file of another is refused.
pub const VERSION: u32 = 1;
/// The header's size in bytes. The file's first byte is the header's, and
/// the header is loaded with the rest: a loaded module is its own record.
pub const HEADER: usize = 72;
/// Where modules are placed: the X51v's port pool
/// (`rp_hal_x51v::devices::PORT_POOL_BASE`, 52 MiB: the RAM pool and the 4
/// MiB below it that sat unused, R298; the runner checks the two agree).
/// The build places them from its first byte; the game's heap is what the
/// set leaves.
pub const POOL: u32 = 0xA8C0_0000;
/// The pool's length in bytes.
pub const POOL_LEN: u32 = 52 * 1024 * 1024;
/// One entry of the by-name table: the name's offset in the names, then
/// the function's address.
pub const EXPORT: usize = 8;

/// XXH32 (Yann Collet's 32-bit xxHash, its published algorithm) of `data`.
#[must_use]
pub fn xxh32(data: &[u8], seed: u32) -> u32 {
    const P1: u32 = 0x9E37_79B1;
    const P2: u32 = 0x85EB_CA77;
    const P3: u32 = 0xC2B2_AE3D;
    const P4: u32 = 0x27D4_EB2F;
    const P5: u32 = 0x1656_67B1;
    fn word(s: &[u8; 16], at: usize) -> u32 {
        u32::from_le_bytes([s[at], s[at + 1], s[at + 2], s[at + 3]])
    }
    fn lane(acc: u32, input: u32) -> u32 {
        acc.wrapping_add(input.wrapping_mul(P2))
            .rotate_left(13)
            .wrapping_mul(P1)
    }
    let (stripes, rest) = data.as_chunks::<16>();
    let mut h = if data.len() >= 16 {
        let mut v = [
            seed.wrapping_add(P1).wrapping_add(P2),
            seed.wrapping_add(P2),
            seed,
            seed.wrapping_sub(P1),
        ];
        for s in stripes {
            v[0] = lane(v[0], word(s, 0));
            v[1] = lane(v[1], word(s, 4));
            v[2] = lane(v[2], word(s, 8));
            v[3] = lane(v[3], word(s, 12));
        }
        v[0].rotate_left(1)
            .wrapping_add(v[1].rotate_left(7))
            .wrapping_add(v[2].rotate_left(12))
            .wrapping_add(v[3].rotate_left(18))
    } else {
        seed.wrapping_add(P5)
    };
    // The length modulo 2^32, as the algorithm has it.
    h = h.wrapping_add(data.len() as u32);
    let (words, tail) = rest.as_chunks::<4>();
    for w in words {
        h = h
            .wrapping_add(u32::from_le_bytes(*w).wrapping_mul(P3))
            .rotate_left(17)
            .wrapping_mul(P4);
    }
    for &b in tail {
        h = h
            .wrapping_add(u32::from(b).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
    }
    h ^= h >> 15;
    h = h.wrapping_mul(P2);
    h ^= h >> 13;
    h = h.wrapping_mul(P3);
    h ^ (h >> 16)
}

/// The runner build's identity (ADR-0039 rule 3): a hash of the addresses
/// it exports, as the table the build links into the runner holds them
/// (one little-endian word a name, in the order of the names sorted). The
/// build computes it from the linked runner's symbols and writes it into
/// every module; the runner computes it from its own table at its start.
#[must_use]
pub fn runner_identity(table: &[u8]) -> u64 {
    (u64::from(xxh32(table, 0x5250_4D44)) << 32) | u64::from(xxh32(table, !0x5250_4D44))
}

/// A module file's header: 18 little-endian words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The runner build it was linked against ([`runner_identity`]).
    pub runner: u64,
    /// The address its first byte (the header's) runs at.
    pub base: u32,
    /// The file's length: the bytes loaded.
    pub file_len: u32,
    /// What it takes of memory from `base`: the loaded bytes, then its
    /// zeroed RAM.
    pub mem_len: u32,
    /// Where the set of modules built with it ends: memory from there is
    /// free (the game's heap).
    pub set_end: u32,
    /// The address of its entry, `int (int argc, char **argv)`; 0 if it is
    /// a library only.
    pub entry: u32,
    /// The constructors of its global objects: the addresses from `.0` to
    /// `.1` hold one function's address a word.
    pub init: (u32, u32),
    /// The address of the by-name table ([`EXPORT`] bytes an entry, sorted
    /// by name) and its entries.
    pub exports: (u32, u32),
    /// The address of the by-address table: one word an entry, the by-name
    /// table's entry numbers sorted by their function's address.
    pub by_address: u32,
    /// The address of the names (each NUL-terminated) and their bytes.
    pub names: (u32, u32),
}

/// Where the check sum is in the header: its last word.
const SUM_AT: usize = HEADER - 4;

fn word_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl Header {
    /// The header's bytes, with `sum` as its check sum.
    #[must_use]
    pub fn to_bytes(&self, sum: u32) -> [u8; HEADER] {
        let words = [
            u32::from_le_bytes(MAGIC),
            VERSION,
            self.runner as u32,
            (self.runner >> 32) as u32,
            self.base,
            self.file_len,
            self.mem_len,
            self.set_end,
            self.entry,
            self.init.0,
            self.init.1,
            self.exports.0,
            self.exports.1,
            self.by_address,
            self.names.0,
            self.names.1,
            0,
            sum,
        ];
        let mut out = [0u8; HEADER];
        for (chunk, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
            *chunk = w.to_le_bytes();
        }
        out
    }

    /// The header in `bytes` and its check sum; `None` if they are not a
    /// module file's first bytes of this version.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<(Self, u32)> {
        let w = |n: usize| word_at(bytes, n * 4);
        if bytes.get(..4)? != MAGIC || w(1)? != VERSION || w(16)? != 0 {
            return None;
        }
        Some((
            Self {
                runner: u64::from(w(2)?) | (u64::from(w(3)?) << 32),
                base: w(4)?,
                file_len: w(5)?,
                mem_len: w(6)?,
                set_end: w(7)?,
                entry: w(8)?,
                init: (w(9)?, w(10)?),
                exports: (w(11)?, w(12)?),
                by_address: w(13)?,
                names: (w(14)?, w(15)?),
            },
            w(17)?,
        ))
    }

    /// Whether everything the header points at lies where it may: inside
    /// the loaded bytes after the header, on word boundaries, the tables
    /// whole. A header that fails this is a damaged file's or a broken
    /// build's; nothing of it is followed.
    #[must_use]
    pub fn holds(&self) -> bool {
        let file_end = u64::from(self.base) + u64::from(self.file_len);
        let inside = |at: u32, len: u64| {
            u64::from(at) >= u64::from(self.base) + HEADER as u64 && u64::from(at) + len <= file_end
        };
        let count = u64::from(self.exports.1);
        self.base.is_multiple_of(4)
            && self.file_len as usize >= HEADER
            && self.mem_len >= self.file_len
            && u64::from(self.base) + u64::from(self.mem_len) <= u64::from(self.set_end)
            && (self.entry == 0 || (self.entry.is_multiple_of(4) && inside(self.entry, 4)))
            && self.init.0.is_multiple_of(4)
            && self.init.1.is_multiple_of(4)
            && self.init.0 <= self.init.1
            && inside(self.init.0, u64::from(self.init.1 - self.init.0))
            && self.exports.0.is_multiple_of(4)
            && inside(self.exports.0, count * EXPORT as u64)
            && self.by_address.is_multiple_of(4)
            && inside(self.by_address, count * 4)
            && inside(self.names.0, u64::from(self.names.1))
    }
}

/// A module file's check sum: over everything but the sum itself, the
/// header first (its sum is the seed of the body's).
#[must_use]
pub fn check_sum(file: &[u8]) -> Option<u32> {
    let (head, body) = (file.get(..SUM_AT)?, file.get(HEADER..)?);
    Some(xxh32(body, xxh32(head, 0)))
}

/// A module's bytes as loaded (its file: `image[0]` is at `header.base`),
/// for its name tables. Every read is bounds-checked: a table that points
/// outside the image answers "not there".
#[derive(Clone, Copy)]
pub struct Names<'a> {
    image: &'a [u8],
    header: Header,
}

impl<'a> Names<'a> {
    /// The tables of `image`, whose header is `header` (one that
    /// [`Header::holds`]).
    #[must_use]
    pub const fn new(image: &'a [u8], header: Header) -> Self {
        Self { image, header }
    }

    fn word(&self, address: u32) -> Option<u32> {
        word_at(self.image, address.checked_sub(self.header.base)? as usize)
    }

    /// Entry `n` of the by-name table: its name's address and the
    /// function's.
    fn entry(&self, n: u32) -> Option<(u32, u32)> {
        if n >= self.header.exports.1 {
            return None;
        }
        let at = self.header.exports.0.checked_add(n.checked_mul(8)?)?;
        let offset = self.word(at)?;
        if offset >= self.header.names.1 {
            return None;
        }
        Some((
            self.header.names.0.checked_add(offset)?,
            self.word(at.checked_add(4)?)?,
        ))
    }

    /// The name at `address`, without its NUL.
    fn name(&self, address: u32) -> Option<&'a [u8]> {
        let from = address.checked_sub(self.header.base)? as usize;
        let end = (self.header.names.0 - self.header.base) as usize + self.header.names.1 as usize;
        let bytes = self.image.get(from..end)?;
        let len = bytes.iter().position(|&b| b == 0)?;
        Some(&bytes[..len])
    }

    /// How many functions the module exports.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.header.exports.1
    }

    /// Whether it exports none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.header.exports.1 == 0
    }

    /// The address of the exported function called `name` (the engine's
    /// `COM_GetProcAddress` and `COM_FunctionFromName`).
    #[must_use]
    pub fn find(&self, name: &[u8]) -> Option<u32> {
        let (mut low, mut high) = (0, self.header.exports.1);
        while low < high {
            let mid = low + (high - low) / 2;
            let (at, function) = self.entry(mid)?;
            match self.name(at)?.cmp(name) {
                core::cmp::Ordering::Equal => return Some(function),
                core::cmp::Ordering::Less => low = mid + 1,
                core::cmp::Ordering::Greater => high = mid,
            }
        }
        None
    }

    /// The exported function that starts at `function`: its name's address
    /// (a NUL-terminated string in the module's own memory) and the name
    /// (the engine's `COM_NameForFunction`).
    #[must_use]
    pub fn name_of(&self, function: u32) -> Option<(u32, &'a [u8])> {
        let (mut low, mut high) = (0, self.header.exports.1);
        while low < high {
            let mid = low + (high - low) / 2;
            let n = self.word(self.header.by_address.checked_add(mid.checked_mul(4)?)?)?;
            let (at, f) = self.entry(n)?;
            match f.cmp(&function) {
                core::cmp::Ordering::Equal => return Some((at, self.name(at)?)),
                core::cmp::Ordering::Less => low = mid + 1,
                core::cmp::Ordering::Greater => high = mid,
            }
        }
        None
    }

    /// Export `n` in the by-name table's order: its name and its
    /// function's address.
    #[must_use]
    pub fn get(&self, n: u32) -> Option<(&'a [u8], u32)> {
        let (at, function) = self.entry(n)?;
        Some((self.name(at)?, function))
    }

    /// Whether every export is found by its name and named by its
    /// address (two names of one function name each other's function):
    /// a whole-table check for a test image, not something a load needs.
    #[must_use]
    pub fn round_trips(&self) -> bool {
        (0..self.len()).all(|n| {
            self.get(n).is_some_and(|(name, function)| {
                self.find(name) == Some(function)
                    && self
                        .name_of(function)
                        .is_some_and(|(_, named)| self.find(named) == Some(function))
            })
        })
    }

    /// Whether both tables are in order and every entry reads: what the
    /// build promises, checked once when a module is loaded so a look-up
    /// never has to doubt it.
    #[must_use]
    pub fn in_order(&self) -> bool {
        let mut last_name: Option<&[u8]> = None;
        let mut last_function = None;
        for n in 0..self.header.exports.1 {
            let Some((at, _)) = self.entry(n) else {
                return false;
            };
            let Some(name) = self.name(at) else {
                return false;
            };
            if last_name.is_some_and(|l| l >= name) {
                return false;
            }
            last_name = Some(name);
            let by = self
                .header
                .by_address
                .checked_add(n * 4)
                .and_then(|a| self.word(a))
                .and_then(|k| self.entry(k));
            let Some((_, function)) = by else {
                return false;
            };
            if last_function.is_some_and(|l| l > function) {
                return false;
            }
            last_function = Some(function);
        }
        true
    }
}

/// Why a module was not loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The file is not there or the card failed.
    Unread,
    /// Not a module file of this format.
    NotAModule,
    /// Linked against another build of the runner.
    OtherRunner,
    /// Its address or size is not memory this runner gives to modules.
    Misplaced,
    /// Its memory is a loaded module's.
    Overlaps,
    /// Its length, check sum or tables do not hold.
    Damaged,
    /// Every place for a loaded module is taken.
    Full,
}

impl Refusal {
    /// The reason in plain words, as a C string too (`rp_module_error`).
    #[must_use]
    pub const fn reason(self) -> &'static core::ffi::CStr {
        match self {
            Self::Unread => c"its file is not there or could not be read",
            Self::NotAModule => c"it is not a RetroPane module file",
            Self::OtherRunner => {
                c"it was built for another build of the runner (the two are built and installed together)"
            }
            Self::Misplaced => c"it was built for memory this runner does not give to modules",
            Self::Overlaps => c"its memory is in use by another loaded module",
            Self::Damaged => c"it is damaged (its length, check sum or tables do not hold)",
            Self::Full => c"too many modules are loaded",
        }
    }
}

/// Modules that may be loaded at once (a game is three; the engine's own
/// parts are inside its module).
pub const LOADED: usize = 8;

/// Reads a module's file from its start: fills as much of the buffer as
/// the file still has and says how many bytes (fewer than asked only at
/// the file's end), or `None` if the read failed.
pub type Read<'r> = &'r mut dyn FnMut(&mut [u8]) -> Option<usize>;

/// Fills `into` from `read`; the bytes got (fewer only at the file's end).
fn fill(read: Read<'_>, into: &mut [u8]) -> Result<usize, Refusal> {
    let mut got = 0;
    while got < into.len() {
        match read(&mut into[got..]) {
            None => return Err(Refusal::Unread),
            Some(0) => break,
            Some(n) => got += n.min(into.len() - got),
        }
    }
    Ok(got)
}

/// The loader: the memory modules are placed in, which runner this is,
/// and what is loaded. Safe code throughout: a module's bytes are written
/// and its tables read through the one slice, bounds-checked; jumping
/// into what it loaded is the C boundary's (`rp_hal_cabi`), after
/// everything here has passed.
pub struct Modules<'a> {
    /// The memory modules are placed in.
    room: &'a mut [u8],
    /// The address of `room[0]`.
    base: u32,
    /// This runner build's identity.
    runner: u64,
    loaded: [Option<Header>; LOADED],
}

impl<'a> Modules<'a> {
    /// A loader placing modules in `room`, whose first byte is at the
    /// address `base`, for the runner build `runner`
    /// ([`runner_identity`]).
    #[must_use]
    pub const fn new(room: &'a mut [u8], base: u32, runner: u64) -> Self {
        Self {
            room,
            base,
            runner,
            loaded: [None; LOADED],
        }
    }

    /// Whether `header` is a module's for this runner, lying in its room.
    fn accept(&self, header: &Header) -> Result<(), Refusal> {
        if header.runner != self.runner {
            return Err(Refusal::OtherRunner);
        }
        if !header.holds() {
            return Err(Refusal::Damaged);
        }
        let end = u64::from(header.base) + u64::from(header.mem_len);
        if header.base < self.base || end > u64::from(self.base) + self.room.len() as u64 {
            return Err(Refusal::Misplaced);
        }
        Ok(())
    }

    /// The header at the start of a module's file, checked as far as a
    /// header alone can be (its format, its runner, its place): for a
    /// runner that must know where the set ends before it loads anything.
    ///
    /// # Errors
    /// Why it is not a module this runner may load.
    pub fn peek(&self, read: Read<'_>) -> Result<Header, Refusal> {
        let mut head = [0u8; HEADER];
        let got = fill(read, &mut head)?;
        let (header, _) = Header::parse(&head[..got]).ok_or(Refusal::NotAModule)?;
        self.accept(&header)?;
        Ok(header)
    }

    /// Loads the module `read` reads: its bytes to its address, checked,
    /// its zeroed RAM zeroed. A module already loaded at that address is
    /// replaced (started afresh). The caller then makes the loaded bytes
    /// fit to run (the caches) and runs its constructors.
    ///
    /// # Errors
    /// Why it was refused; nothing of a refused module is ever run, and
    /// no module is left loaded at its address.
    pub fn load(&mut self, read: Read<'_>) -> Result<Header, Refusal> {
        let mut head = [0u8; HEADER];
        let got = fill(read, &mut head)?;
        let (header, sum) = Header::parse(&head[..got]).ok_or(Refusal::NotAModule)?;
        self.accept(&header)?;
        let (from, to) = (
            u64::from(header.base),
            u64::from(header.base) + u64::from(header.mem_len),
        );
        let mut slot = None;
        for (i, l) in self.loaded.iter().enumerate() {
            match l {
                Some(l) if l.base == header.base => slot = Some(i),
                Some(l)
                    if u64::from(l.base) < to
                        && from < u64::from(l.base) + u64::from(l.mem_len) =>
                {
                    return Err(Refusal::Overlaps);
                }
                None if slot.is_none() => slot = Some(i),
                _ => {}
            }
        }
        // A module being replaced may also have grown over a neighbour:
        // the loop above checked every other one.
        let slot = slot.ok_or(Refusal::Full)?;
        // From here its memory is written: nothing is loaded there until
        // every check has passed.
        self.loaded[slot] = None;
        let at = (header.base - self.base) as usize;
        let memory = &mut self.room[at..at + header.mem_len as usize];
        let (file, zeroed) = memory.split_at_mut(header.file_len as usize);
        file[..HEADER].copy_from_slice(&head);
        if fill(read, &mut file[HEADER..])? != file.len() - HEADER {
            return Err(Refusal::Damaged);
        }
        // The file must end where its header says.
        let mut more = [0u8; 1];
        if fill(read, &mut more)? != 0 {
            return Err(Refusal::Damaged);
        }
        if check_sum(file) != Some(sum) {
            return Err(Refusal::Damaged);
        }
        let names = Names::new(file, header);
        if !names.in_order() {
            return Err(Refusal::Damaged);
        }
        // Every constructor is code of this module's.
        let code = header.base + HEADER as u32..header.base + header.file_len;
        let mut c = header.init.0;
        while c < header.init.1 {
            match names.word(c) {
                Some(f) if f.is_multiple_of(4) && code.contains(&f) => {}
                _ => return Err(Refusal::Damaged),
            }
            c += 4;
        }
        zeroed.fill(0);
        self.loaded[slot] = Some(header);
        Ok(header)
    }

    /// The loaded module whose handle (its address) is `handle`.
    #[must_use]
    pub fn header(&self, handle: u32) -> Option<Header> {
        self.loaded
            .iter()
            .flatten()
            .find(|h| h.base == handle)
            .copied()
    }

    /// The name tables of the loaded module `handle`.
    #[must_use]
    pub fn names(&self, handle: u32) -> Option<Names<'_>> {
        let header = self.header(handle)?;
        let at = (header.base - self.base) as usize;
        Some(Names::new(
            self.room.get(at..at + header.file_len as usize)?,
            header,
        ))
    }

    /// Forgets the module `handle`; whether it was loaded. Its memory
    /// stays its own: the build placed nothing else there.
    pub fn free(&mut self, handle: u32) -> bool {
        match self
            .loaded
            .iter_mut()
            .find(|l| l.is_some_and(|h| h.base == handle))
        {
            Some(l) => {
                *l = None;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A module file made by hand: `code` bytes of filler after the
    /// header, then the tables for `exports` (name, address), the
    /// constructors' words, and `zeroed` bytes of zeroed RAM asked for.
    pub fn module(
        runner: u64,
        base: u32,
        code: usize,
        exports: &[(&str, u32)],
        constructors: &[u32],
        zeroed: u32,
    ) -> Vec<u8> {
        let mut sorted: Vec<(&str, u32)> = exports.to_vec();
        sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut file = vec![0u8; HEADER];
        file.extend((0..code.next_multiple_of(4)).map(|i| (i * 7 + 1) as u8));
        let at = |file: &Vec<u8>| base + file.len() as u32;
        let mut names = Vec::new();
        let exports_at = at(&file);
        for (name, function) in &sorted {
            file.extend((names.len() as u32).to_le_bytes());
            file.extend(function.to_le_bytes());
            names.extend(name.as_bytes());
            names.push(0);
        }
        let by_address = at(&file);
        let mut order: Vec<u32> = (0..sorted.len() as u32).collect();
        order.sort_by_key(|&n| sorted[n as usize].1);
        for n in order {
            file.extend(n.to_le_bytes());
        }
        let names_at = at(&file);
        file.extend(&names);
        file.resize(file.len().next_multiple_of(4), 0);
        let init = at(&file);
        for c in constructors {
            file.extend(c.to_le_bytes());
        }
        let init_end = at(&file);
        let header = Header {
            runner,
            base,
            file_len: file.len() as u32,
            mem_len: file.len() as u32 + zeroed,
            set_end: (base + file.len() as u32 + zeroed).next_multiple_of(4096),
            entry: base + HEADER as u32,
            init: (init, init_end),
            exports: (exports_at, sorted.len() as u32),
            by_address,
            names: (names_at, names.len() as u32),
        };
        seal(&mut file, &header);
        file
    }

    /// Writes `header` and the check sum into `file`.
    pub fn seal(file: &mut [u8], header: &Header) {
        file[..HEADER].copy_from_slice(&header.to_bytes(0));
        let sum = check_sum(file).expect("a whole header");
        file[..HEADER].copy_from_slice(&header.to_bytes(sum));
    }
}

#[cfg(test)]
mod tests {
    use super::testing::module;
    use super::*;

    #[test]
    fn the_check_sum_is_xxh32_as_published() {
        // The algorithm's own check values (xxHash's documentation and the
        // reference implementation's self-test).
        assert_eq!(xxh32(b"", 0), 0x02CC_5D05);
        assert_eq!(xxh32(b"", 1), 0x0B2C_B792);
        assert_eq!(xxh32(b"a", 0), 0x550D_7456);
        assert_eq!(xxh32(b"abc", 0), 0x32D1_53FF);
        // 39 bytes: two 16-byte stripes, a word, three bytes.
        assert_eq!(
            xxh32(b"Nobody inspects the spammish repetition", 0),
            0xE229_3B2F
        );
        // One bit anywhere changes it, and so does the length.
        let mut data = [0x5Au8; 100];
        let whole = xxh32(&data, 0);
        for at in [0, 15, 16, 63, 96, 99] {
            data[at] ^= 1;
            assert_ne!(xxh32(&data, 0), whole, "byte {at}");
            data[at] ^= 1;
        }
        assert_ne!(xxh32(&data[..99], 0), whole);
        assert_ne!(xxh32(&[0u8; 16], 0), xxh32(&[0u8; 17], 0));
    }

    #[test]
    fn the_runner_s_identity_follows_every_address() {
        let table = |a: &[u32]| -> Vec<u8> { a.iter().flat_map(|w| w.to_le_bytes()).collect() };
        let id = runner_identity(&table(&[0xA800_8000, 0xA800_9000, 0xA801_0000]));
        // One function four bytes on, two swapped, one more or one fewer.
        assert_ne!(
            id,
            runner_identity(&table(&[0xA800_8000, 0xA800_9004, 0xA801_0000]))
        );
        assert_ne!(
            id,
            runner_identity(&table(&[0xA800_9000, 0xA800_8000, 0xA801_0000]))
        );
        assert_ne!(id, runner_identity(&table(&[0xA800_8000, 0xA800_9000])));
        assert_ne!(runner_identity(&[]), runner_identity(&table(&[0])));
        // Both halves carry it.
        assert_ne!(id >> 32, id & 0xFFFF_FFFF);
    }

    #[test]
    fn a_header_reads_back_as_written() {
        let h = Header {
            runner: 0x0123_4567_89AB_CDEF,
            base: 0xA900_0000,
            file_len: 0x1000,
            mem_len: 0x3000,
            set_end: 0xA900_4000,
            entry: 0xA900_0048,
            init: (0xA900_0800, 0xA900_0810),
            exports: (0xA900_0900, 3),
            by_address: 0xA900_0918,
            names: (0xA900_0924, 40),
        };
        assert!(h.holds());
        let bytes = h.to_bytes(0xDEAD_BEEF);
        assert_eq!(&bytes[..4], b"RPMD");
        assert_eq!(Header::parse(&bytes), Some((h, 0xDEAD_BEEF)));
        // Not a module: another magic, another version, a word that must
        // be zero, too few bytes.
        let mut other = bytes;
        other[0] = b'X';
        assert_eq!(Header::parse(&other), None);
        let mut other = bytes;
        other[4] = 2;
        assert_eq!(Header::parse(&other), None);
        let mut other = bytes;
        other[64] = 1;
        assert_eq!(Header::parse(&other), None);
        assert_eq!(Header::parse(&bytes[..HEADER - 1]), None);
    }

    #[test]
    fn a_header_pointing_outside_its_file_does_not_hold() {
        let good = Header {
            runner: 1,
            base: 0xA900_0000,
            file_len: 0x1000,
            mem_len: 0x1000,
            set_end: 0xA900_1000,
            entry: 0,
            init: (0xA900_0048, 0xA900_0048),
            exports: (0xA900_0048, 0),
            by_address: 0xA900_0048,
            names: (0xA900_0048, 0),
        };
        assert!(good.holds());
        let bad = |change: fn(&mut Header)| {
            let mut h = good;
            change(&mut h);
            !h.holds()
        };
        // Each limit, one step past it.
        assert!(bad(|h| h.base += 2));
        assert!(bad(|h| h.file_len = HEADER as u32 - 4));
        assert!(bad(|h| h.mem_len = h.file_len - 4));
        assert!(bad(|h| h.set_end -= 4));
        assert!(bad(|h| h.entry = h.base));
        assert!(bad(|h| h.entry = h.base + h.file_len));
        assert!(bad(|h| h.entry = h.base + HEADER as u32 + 2));
        assert!(bad(|h| h.init = (h.init.0, h.base + h.file_len + 4)));
        assert!(bad(|h| h.init = (h.init.0 + 8, h.init.0)));
        assert!(bad(|h| h.exports = (h.base + h.file_len - 4, 1)));
        assert!(bad(|h| h.exports.1 = u32::MAX));
        assert!(bad(|h| h.by_address = h.base + h.file_len + 4));
        assert!(bad(|h| h.by_address = h.base));
        assert!(bad(|h| h.names = (h.base + h.file_len - 1, 2)));
        assert!(bad(|h| h.names = (0xFFFF_FFFF, 0xFFFF_FFFF)));
        // The last word of the file is still inside it.
        let mut h = good;
        h.entry = h.base + h.file_len - 4;
        h.exports = (h.base + h.file_len - 8, 1);
        assert!(h.holds());
    }

    #[test]
    fn a_name_is_found_and_an_address_named() {
        let base = 0xA900_0000;
        let file = module(
            7,
            base,
            64,
            &[
                ("monster_zombie", base + 0x60),
                ("GiveFnptrsToDll", base + 0x50),
                ("_ZN5CBase5ThinkEv", base + 0x58),
                ("worldspawn", base + 0x48),
                ("monster", base + 0x70),
            ],
            &[],
            0,
        );
        let (header, sum) = Header::parse(&file).expect("a header");
        assert!(header.holds());
        assert_eq!(check_sum(&file), Some(sum));
        let names = Names::new(&file, header);
        assert_eq!(names.len(), 5);
        assert!(names.in_order());
        assert_eq!(names.find(b"worldspawn"), Some(base + 0x48));
        assert_eq!(names.find(b"GiveFnptrsToDll"), Some(base + 0x50));
        assert_eq!(names.find(b"monster"), Some(base + 0x70));
        assert_eq!(names.find(b"monster_zombie"), Some(base + 0x60));
        // A name's start, its longer neighbour, another case, nothing.
        assert_eq!(names.find(b"monster_"), None);
        assert_eq!(names.find(b"monster_zombies"), None);
        assert_eq!(names.find(b"WorldSpawn"), None);
        assert_eq!(names.find(b""), None);
        // The other way: only a function's own first address has a name.
        let (at, name) = names.name_of(base + 0x58).expect("named");
        assert_eq!(name, b"_ZN5CBase5ThinkEv");
        let from = (at - base) as usize;
        assert_eq!(&file[from..from + 18], b"_ZN5CBase5ThinkEv\0");
        assert_eq!(
            names.name_of(base + 0x48).map(|n| n.1),
            Some(&b"worldspawn"[..])
        );
        assert_eq!(
            names.name_of(base + 0x70).map(|n| n.1),
            Some(&b"monster"[..])
        );
        assert_eq!(names.name_of(base + 0x5C), None);
        assert_eq!(names.name_of(0), None);
        // In the by-name order, and the whole table both ways.
        assert_eq!(names.get(0), Some((&b"GiveFnptrsToDll"[..], base + 0x50)));
        assert_eq!(names.get(4), Some((&b"worldspawn"[..], base + 0x48)));
        assert_eq!(names.get(5), None);
        assert!(names.round_trips());
        assert_eq!(names.name_of(u32::MAX), None);
    }

    #[test]
    fn a_module_with_no_exports_answers_nothing() {
        let file = module(7, 0xA900_0000, 16, &[], &[], 0);
        let (header, _) = Header::parse(&file).expect("a header");
        let names = Names::new(&file, header);
        assert!(names.is_empty() && names.in_order());
        assert_eq!(names.find(b"anything"), None);
        assert_eq!(names.name_of(0xA900_0048), None);
    }

    #[test]
    fn tables_out_of_order_or_pointing_away_are_seen() {
        let base = 0xA900_0000;
        let file = module(
            7,
            base,
            16,
            &[("a", base + 0x48), ("b", base + 0x4C)],
            &[],
            0,
        );
        let (header, _) = Header::parse(&file).expect("a header");
        assert!(Names::new(&file, header).in_order());
        let table = (header.exports.0 - base) as usize;
        // The two names swapped: no longer sorted, and "a" is no longer
        // found where the search looks for it.
        let mut bad = file.clone();
        bad[table..table + 4].copy_from_slice(&2u32.to_le_bytes());
        bad[table + 8..table + 12].copy_from_slice(&0u32.to_le_bytes());
        assert!(!Names::new(&bad, header).in_order());
        assert!(Names::new(&file, header).round_trips());
        assert!(!Names::new(&bad, header).round_trips());
        // A name offset past the names.
        let mut bad = file.clone();
        bad[table + 8..table + 12].copy_from_slice(&400u32.to_le_bytes());
        assert!(!Names::new(&bad, header).in_order());
        assert_eq!(Names::new(&bad, header).find(b"b"), None);
        // The by-address table naming an entry that is not there, or out
        // of order.
        let by = (header.by_address - base) as usize;
        let mut bad = file.clone();
        bad[by..by + 4].copy_from_slice(&9u32.to_le_bytes());
        assert!(!Names::new(&bad, header).in_order());
        let mut bad = file.clone();
        bad[by..by + 4].copy_from_slice(&1u32.to_le_bytes());
        bad[by + 4..by + 8].copy_from_slice(&0u32.to_le_bytes());
        assert!(!Names::new(&bad, header).in_order());
        // The last name's NUL cut off: it does not read.
        let mut short = header;
        short.names.1 -= 1;
        assert!(!Names::new(&file, short).in_order());
    }
}

#[cfg(test)]
mod loader_tests {
    use super::testing::{module, seal};
    use super::*;

    const BASE: u32 = 0xA900_0000;
    const RUNNER: u64 = 0x1122_3344_5566_7788;
    const ROOM: usize = 64 * 1024;

    /// Reads `file` at most `piece` bytes a call, as a card does.
    fn reader(file: &[u8], piece: usize) -> impl FnMut(&mut [u8]) -> Option<usize> + '_ {
        let mut at = 0;
        move |into: &mut [u8]| {
            let n = into.len().min(piece).min(file.len() - at);
            into[..n].copy_from_slice(&file[at..at + n]);
            at += n;
            Some(n)
        }
    }

    fn load(m: &mut Modules<'_>, file: &[u8]) -> Result<Header, Refusal> {
        m.load(&mut reader(file, 4096))
    }

    /// A module at `base` exporting two functions, with a constructor and
    /// 256 bytes of zeroed RAM.
    fn sample(base: u32) -> Vec<u8> {
        module(
            RUNNER,
            base,
            200,
            &[("GetEntityAPI", base + 0x50), ("worldspawn", base + 0x80)],
            &[base + 0x60],
            256,
        )
    }

    #[test]
    fn a_module_is_read_to_its_address_checked_and_its_zeroed_ram_zeroed() {
        let file = sample(BASE + 0x1000);
        // In pieces of every awkward size, into memory full of old bytes.
        for piece in [1, 7, 71, 72, 73, 4096] {
            let mut room = vec![0xEEu8; ROOM];
            let mut m = Modules::new(&mut room, BASE, RUNNER);
            let h = m
                .load(&mut reader(&file, piece))
                .unwrap_or_else(|e| panic!("piece {piece}: {e:?}"));
            assert_eq!(h.base, BASE + 0x1000);
            assert_eq!(h.file_len as usize, file.len());
            assert_eq!(m.header(BASE + 0x1000), Some(h));
            let names = m.names(BASE + 0x1000).expect("loaded");
            assert_eq!(names.find(b"worldspawn"), Some(BASE + 0x1080));
            assert_eq!(
                names.name_of(BASE + 0x1050).map(|n| n.1),
                Some(&b"GetEntityAPI"[..])
            );
            let _ = m;
            // The file where it runs, its zeroed RAM zero, not a byte more
            // touched on either side.
            assert_eq!(&room[0x1000..0x1000 + file.len()], &file[..]);
            let end = 0x1000 + file.len();
            assert!(room[end..end + 256].iter().all(|&b| b == 0));
            assert_eq!(room[end + 256], 0xEE);
            assert_eq!(room[0x0FFF], 0xEE);
        }
    }

    #[test]
    fn what_is_not_a_module_or_is_another_runner_s_is_refused_before_a_byte_is_written() {
        let file = sample(BASE);
        let refused = |file: &[u8], runner: u64| {
            let mut room = vec![0xEEu8; ROOM];
            let mut m = Modules::new(&mut room, BASE, runner);
            let r = load(&mut m, file).expect_err("refused");
            assert_eq!(m.header(BASE), None);
            assert!(m.names(BASE).is_none());
            let _ = m;
            assert!(room.iter().all(|&b| b == 0xEE), "{r:?} wrote memory");
            r
        };
        assert_eq!(refused(b"", RUNNER), Refusal::NotAModule);
        assert_eq!(
            refused(b"MZ\x90\x00 some other file", RUNNER),
            Refusal::NotAModule
        );
        assert_eq!(refused(&file[..HEADER - 1], RUNNER), Refusal::NotAModule);
        // The same module under a runner one address different.
        assert_eq!(refused(&file, RUNNER + 1), Refusal::OtherRunner);
        // A card that fails.
        let mut room = vec![0u8; ROOM];
        let mut m = Modules::new(&mut room, BASE, RUNNER);
        assert_eq!(m.load(&mut |_: &mut [u8]| None), Err(Refusal::Unread));
        // Every reason is words a person can read.
        for r in [
            Refusal::Unread,
            Refusal::NotAModule,
            Refusal::OtherRunner,
            Refusal::Misplaced,
            Refusal::Overlaps,
            Refusal::Damaged,
            Refusal::Full,
        ] {
            assert!(r.reason().to_bytes().len() > 20, "{r:?}");
        }
    }

    #[test]
    fn a_module_for_memory_outside_the_room_is_refused() {
        let place = |base: u32, zeroed: u32| {
            let file = module(RUNNER, base, 100, &[], &[], zeroed);
            let mut room = vec![0u8; ROOM];
            let mut m = Modules::new(&mut room, BASE, RUNNER);
            load(&mut m, &file).map(|h| h.base + h.mem_len)
        };
        // Below the room, past its end, its zeroed RAM past its end.
        assert_eq!(place(BASE - 0x1000, 0), Err(Refusal::Misplaced));
        assert_eq!(place(BASE + ROOM as u32, 0), Err(Refusal::Misplaced));
        assert_eq!(
            place(BASE + ROOM as u32 - 0x100, 0x100),
            Err(Refusal::Misplaced)
        );
        assert_eq!(place(0xFFFF_0000, 0), Err(Refusal::Misplaced));
        // To the last byte fits; one word more does not.
        let file_len = module(RUNNER, BASE, 100, &[], &[], 0).len() as u32;
        let last = BASE + ROOM as u32 - 0x1000;
        assert_eq!(place(last, 0x1000 - file_len), Ok(BASE + ROOM as u32));
        assert_eq!(place(last, 0x1000 - file_len + 4), Err(Refusal::Misplaced));
    }

    #[test]
    fn a_damaged_file_is_refused_wherever_the_damage_is() {
        let file = sample(BASE);
        let refused = |file: &[u8]| {
            let mut room = vec![0u8; ROOM];
            let mut m = Modules::new(&mut room, BASE, RUNNER);
            let r = load(&mut m, file);
            assert_eq!(m.header(BASE), None);
            r
        };
        // One bit, in the code, a table, the names, the last byte.
        for at in [
            HEADER,
            HEADER + 97,
            file.len() / 2,
            file.len() - 9,
            file.len() - 1,
        ] {
            let mut bad = file.clone();
            bad[at] ^= 0x10;
            assert_eq!(refused(&bad), Err(Refusal::Damaged), "byte {at}");
        }
        // A header field changed (the sum covers the header).
        let mut bad = file.clone();
        bad[32] ^= 4; // the entry
        assert_eq!(refused(&bad), Err(Refusal::Damaged));
        // Cut short, by a byte and by most of it; a byte too long.
        assert_eq!(refused(&file[..file.len() - 1]), Err(Refusal::Damaged));
        assert_eq!(refused(&file[..HEADER + 3]), Err(Refusal::Damaged));
        let mut long = file.clone();
        long.push(0);
        assert_eq!(refused(&long), Err(Refusal::Damaged));
        // A header that points outside its file, sealed with a good sum.
        let (mut header, _) = Header::parse(&file).expect("a header");
        header.exports.1 = 4000;
        let mut bad = file.clone();
        seal(&mut bad, &header);
        assert_eq!(refused(&bad), Err(Refusal::Damaged));
        // A constructor that is not this module's code, sealed the same.
        for target in [BASE, BASE + 0x10_0000, BASE + 0x62, 0] {
            let mut bad = module(RUNNER, BASE, 200, &[], &[BASE + 0x60, target], 0);
            let (header, _) = Header::parse(&bad).expect("a header");
            seal(&mut bad, &header);
            assert_eq!(refused(&bad), Err(Refusal::Damaged), "{target:#x}");
        }
        // Names out of order, sealed with a good sum: still refused.
        let mut bad = file.clone();
        let (header, _) = Header::parse(&file).expect("a header");
        let table = (header.exports.0 - BASE) as usize;
        let (a, b) = (
            bad[table..table + 8].to_vec(),
            bad[table + 8..table + 16].to_vec(),
        );
        bad[table..table + 8].copy_from_slice(&b);
        bad[table + 8..table + 16].copy_from_slice(&a);
        seal(&mut bad, &header);
        assert_eq!(refused(&bad), Err(Refusal::Damaged));
        // And the file as built loads.
        let mut room = vec![0u8; ROOM];
        assert!(load(&mut Modules::new(&mut room, BASE, RUNNER), &file).is_ok());
    }

    #[test]
    fn modules_load_side_by_side_never_over_each_other_and_start_afresh() {
        let first = sample(BASE);
        let second = sample(BASE + 0x2000);
        let mut room = vec![0xEEu8; ROOM];
        let mut m = Modules::new(&mut room, BASE, RUNNER);
        load(&mut m, &first).expect("the first");
        load(&mut m, &second).expect("beside it");
        // Each answers for itself.
        assert_eq!(
            m.names(BASE).and_then(|n| n.find(b"worldspawn")),
            Some(BASE + 0x80)
        );
        assert_eq!(
            m.names(BASE + 0x2000).and_then(|n| n.find(b"worldspawn")),
            Some(BASE + 0x2080)
        );
        assert!(m.names(BASE + 0x1000).is_none());
        // One built over the first's memory (its last byte) is refused and
        // the first stays as it was.
        let end = first.len() as u32 + 256;
        let over = module(RUNNER, BASE + end - 4, 100, &[], &[], 0);
        assert_eq!(load(&mut m, &over), Err(Refusal::Overlaps));
        // One that would reach into the second from below, too.
        let under = module(RUNNER, BASE + 0x1000, 100, &[], &[], 0x1000);
        assert_eq!(load(&mut m, &under), Err(Refusal::Overlaps));
        assert_eq!(
            m.names(BASE).and_then(|n| n.find(b"GetEntityAPI")),
            Some(BASE + 0x50)
        );
        // Right behind the first fits.
        let behind = module(RUNNER, BASE + end, 100, &[], &[], 0);
        load(&mut m, &behind).expect("behind the first");
        // Loaded again, the first starts afresh: what the game changed in
        // its data and its zeroed RAM is gone.
        let _ = m;
        room[HEADER + 8] ^= 0xFF;
        room[first.len() + 10] = 0x55;
        let mut m = Modules::new(&mut room, BASE, RUNNER);
        load(&mut m, &first).expect("again");
        load(&mut m, &first).expect("and over itself");
        let _ = m;
        assert_eq!(&room[..first.len()], &first[..]);
        assert_eq!(room[first.len() + 10], 0);
    }

    #[test]
    fn a_refused_reload_leaves_nothing_loaded_and_free_forgets() {
        let file = sample(BASE);
        let mut room = vec![0u8; ROOM];
        let mut m = Modules::new(&mut room, BASE, RUNNER);
        load(&mut m, &file).expect("loaded");
        let mut bad = file.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(load(&mut m, &bad), Err(Refusal::Damaged));
        // Its memory holds the damaged bytes: it is not a loaded module.
        assert_eq!(m.header(BASE), None);
        assert!(m.names(BASE).is_none());
        assert!(!m.free(BASE));
        load(&mut m, &file).expect("the good file again");
        assert!(m.free(BASE));
        assert!(!m.free(BASE));
        assert!(m.names(BASE).is_none());
    }

    #[test]
    fn there_is_room_for_eight_modules_and_the_ninth_is_refused() {
        let mut room = vec![0u8; ROOM];
        let mut m = Modules::new(&mut room, BASE, RUNNER);
        for i in 0..LOADED as u32 {
            let file = module(RUNNER, BASE + i * 0x1000, 100, &[], &[], 0);
            load(&mut m, &file).unwrap_or_else(|e| panic!("module {i}: {e:?}"));
        }
        let ninth = module(RUNNER, BASE + 0x8000, 100, &[], &[], 0);
        assert_eq!(load(&mut m, &ninth), Err(Refusal::Full));
        // One freed, there is room again; a reload needs none.
        assert!(m.free(BASE + 0x3000));
        load(&mut m, &ninth).expect("in the freed place");
        let again = module(RUNNER, BASE, 100, &[], &[], 0);
        load(&mut m, &again).expect("a reload");
    }

    #[test]
    fn a_header_alone_says_where_the_set_ends() {
        let file = sample(BASE + 0x1000);
        let mut room = vec![0xEEu8; ROOM];
        let m = Modules::new(&mut room, BASE, RUNNER);
        let h = m.peek(&mut reader(&file, 5)).expect("a header");
        assert_eq!(h.base, BASE + 0x1000);
        assert_eq!(h.set_end % 4096, 0);
        assert!(h.set_end >= h.base + h.mem_len);
        assert_eq!(m.peek(&mut reader(b"not one", 5)), Err(Refusal::NotAModule));
        let other = Modules::new(&mut [], BASE, RUNNER ^ 1);
        assert_eq!(
            other.peek(&mut reader(&file, 99)),
            Err(Refusal::OtherRunner)
        );
        let small = Modules::new(&mut [], BASE, RUNNER);
        assert_eq!(small.peek(&mut reader(&file, 99)), Err(Refusal::Misplaced));
        // Nothing was written.
        let _ = m;
        assert!(room.iter().all(|&b| b == 0xEE));
    }
}
