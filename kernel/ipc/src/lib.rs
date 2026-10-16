//! Service calls between processes (ADR-0016): the rules, separate from the
//! context switch that carries them out.
//!
//! A server process opens a **port** (an entry point in its slot). Other
//! processes reach it only through a **handle** in their own handle table,
//! with **rights**: `CALL` to call it, `GRANT` to hand it on (only ever with
//! the same rights or fewer). A call **migrates** the calling thread into
//! the server's slot: [`Ipc::call`] checks everything and counts the
//! nesting, [`Ipc::ret`] uncounts it; a server that faults mid-call unwinds
//! to its caller ([`Ipc::fault`]), which gets an error. Where to come back
//! to (the caller's slot and registers) is the kernel's: it keeps them in
//! the caller's own kernel stack frame, so no table here holds them.
//! Restarting a server bumps its port's **generation**, so handles taken
//! before the restart fail cleanly instead of reaching the new instance.
//!
//! Security (docs/security.md): a handle is an index into the caller's own
//! table, so a process can't name another's handles; forged or stale
//! numbers fail; rights can't grow; nesting is bounded. A handle table
//! starts with [`INLINE_HANDLES`] entries in the process's record and grows
//! into kernel pages as the process takes more ([`HandlePages`]), up to a
//! per-process quota ([`HANDLE_PAGES`]), so nothing an attacker does grows
//! kernel memory without bound, and one process can't take another's room.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Ports in the system.
pub const MAX_PORTS: usize = 64;
/// Handles kept in each process's record: enough for every service so far
/// (a client holds one or two), so most processes never need a page.
pub const INLINE_HANDLES: usize = 8;
/// Handles in one 4 KiB page (a word each).
pub const PER_PAGE: usize = 1024;
/// Pages a process's table may grow into: its quota.
pub const HANDLE_PAGES: usize = 4;
/// Handles a process may hold.
pub const MAX_HANDLES: usize = INLINE_HANDLES + HANDLE_PAGES * PER_PAGE;
/// CE processes' slots (`rp_kernel_mm::layout`, CE5's layout: slot 0 is
/// the running process's alias, slot 1 shared code, and above slot 32 lies
/// the shared-file area, so 31). The kernel checks at build time that these
/// match the layout.
pub const FIRST_SLOT: u8 = 2;
/// See [`FIRST_SLOT`].
pub const LAST_SLOT: u8 = 32;
/// Native processes' slots, above the kernel (`0xC0000000`): 31 more.
pub const FIRST_NATIVE_SLOT: u8 = 96;
/// See [`FIRST_NATIVE_SLOT`].
pub const LAST_NATIVE_SLOT: u8 = 126;
/// Process slots, so handle tables: one per process, none for slots that
/// can't hold one.
pub const MAX_SLOTS: usize =
    (LAST_SLOT - FIRST_SLOT + 1 + LAST_NATIVE_SLOT - FIRST_NATIVE_SLOT + 1) as usize;
/// Threads, numbered 1 to this as the scheduler numbers them.
pub const MAX_THREADS: usize = 255;
/// Nested calls a thread may be in (a service calling a service...).
pub const MAX_DEPTH: usize = 4;

/// What a handle allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rights(u8);

impl Rights {
    /// Call the port.
    pub const CALL: Self = Self(1);
    /// Hand the port on to another process.
    pub const GRANT: Self = Self(2);
    /// Both.
    pub const ALL: Self = Self(3);

    /// Both sets' rights.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Rights from their bits (`CALL` 1, `GRANT` 2; others ignored): for
    /// rights a process asks for in a system call.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self((bits & 3) as u8)
    }

    /// Whether every right in `other` is in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A port's number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortId(u8);

/// A handle: an index into the calling process's own table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(pub u16);

/// Where handle pages come from: kernel-private 4 KiB pages from the frame
/// allocator, never mapped for a process. The same contract as the region
/// lists' pages (`rp_kernel_mm::regions::RegionPages`), so the kernel serves
/// both from one allocator. A page's contents when allocated don't matter:
/// it is cleared here.
pub trait HandlePages {
    /// A free page's address, or `None` when memory is out.
    fn alloc_page(&mut self) -> Option<u32>;
    /// Gives a page back.
    fn free_page(&mut self, page: u32);
    /// The page's bytes.
    fn page(&mut self, page: u32) -> &mut [u8; 4096];
}

/// One process's handle table: [`INLINE_HANDLES`] words in the record,
/// then up to [`HANDLE_PAGES`] pages (0: none yet). All zero when empty.
#[derive(Clone, Copy)]
struct Table {
    inline: [u32; INLINE_HANDLES],
    pages: [u32; HANDLE_PAGES],
}

const EMPTY_TABLE: Table = Table {
    inline: [0; INLINE_HANDLES],
    pages: [0; HANDLE_PAGES],
};

impl Table {
    /// The word for handle `h`, if the table reaches that far.
    fn get(&self, hp: &mut (impl HandlePages + ?Sized), h: usize) -> Option<u32> {
        if h < INLINE_HANDLES {
            return Some(self.inline[h]);
        }
        let (p, w) = (
            (h - INLINE_HANDLES) / PER_PAGE,
            (h - INLINE_HANDLES) % PER_PAGE,
        );
        let page = *self.pages.get(p).filter(|&&a| a != 0)?;
        let b = &hp.page(page)[w * 4..w * 4 + 4];
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Writes handle `h`'s word (the table reaches that far).
    fn set(&mut self, hp: &mut (impl HandlePages + ?Sized), h: usize, word: u32) {
        if h < INLINE_HANDLES {
            self.inline[h] = word;
            return;
        }
        let (p, w) = (
            (h - INLINE_HANDLES) / PER_PAGE,
            (h - INLINE_HANDLES) % PER_PAGE,
        );
        hp.page(self.pages[p])[w * 4..w * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }

    /// Handles the table reaches now.
    fn reach(&self) -> usize {
        INLINE_HANDLES + self.pages.iter().take_while(|&&a| a != 0).count() * PER_PAGE
    }

    /// A free handle number, growing into a new page if every one is taken.
    fn free_slot(&mut self, hp: &mut (impl HandlePages + ?Sized)) -> Result<usize, Error> {
        let reach = self.reach();
        if let Some(h) = (0..reach).find(|&h| self.get(hp, h) == Some(Entry::EMPTY)) {
            return Ok(h);
        }
        let p = (reach - INLINE_HANDLES) / PER_PAGE;
        if p >= HANDLE_PAGES {
            return Err(Error::Full);
        }
        let page = hp.alloc_page().ok_or(Error::Full)?;
        hp.page(page).fill(0);
        self.pages[p] = page;
        Ok(reach)
    }

    /// Calls `f` on every handle word, with a chance to replace it.
    fn update(&mut self, hp: &mut (impl HandlePages + ?Sized), mut f: impl FnMut(u32) -> u32) {
        for w in &mut self.inline {
            *w = f(*w);
        }
        for &page in self.pages.iter().take_while(|&&a| a != 0) {
            for b in hp.page(page).as_chunks_mut::<4>().0 {
                let w = u32::from_le_bytes(*b);
                let n = f(w);
                if n != w {
                    *b = n.to_le_bytes();
                }
            }
        }
    }

    /// Empties the table and gives its pages back.
    fn clear(&mut self, hp: &mut (impl HandlePages + ?Sized)) {
        for &page in self.pages.iter().take_while(|&&a| a != 0) {
            hp.free_page(page);
        }
        *self = EMPTY_TABLE;
    }
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a handle in this process's table.
    BadHandle,
    /// The handle lacks the right.
    NoRight,
    /// The port was closed (its server exited).
    Closed,
    /// The port's server was restarted since this handle was taken.
    Stale,
    /// Calls nested too deep.
    TooDeep,
    /// Not inside a call (a return with nothing to return to).
    NotInCall,
    /// A table is full.
    Full,
    /// A slot or thread number out of range.
    Range,
}

/// A port. All zero means never used (no server is slot 0), so the table
/// needs no `Option` (8 bytes a port, not 12) and starts zeroed (`.bss`).
#[derive(Clone, Copy)]
struct Port {
    entry: u32,
    generation: u16,
    server: u8,
    /// 1 while open. A byte, not a `bool`: a `bool` niche once moved every
    /// `Ipc` static into the image instead of zeroed RAM (13.7 KB,
    /// measured).
    open: u8,
}

const NO_PORT: Port = Port {
    entry: 0,
    generation: 0,
    server: 0,
    open: 0,
};

/// A handle table entry, stored as one word: 0 when empty, else bit 31
/// set, the generation in bits 8..24, the rights in bits 6..8 and the port
/// (64 ports) in bits 0..6. No `Option` and no padding: a table's inline
/// part is all zero at start (so in `.bss`, not the image), and a page is
/// plain words.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Entry(u32);

impl Entry {
    const EMPTY: u32 = 0;
    const USED: u32 = 1 << 31;

    const fn new(port: u8, rights: Rights, generation: u16) -> Self {
        Self(
            Self::USED
                | (generation as u32) << 8
                | ((rights.0 & 3) as u32) << 6
                | (port & 0x3F) as u32,
        )
    }

    /// The entry a table word holds, if any.
    const fn decode(word: u32) -> Option<Self> {
        if word & Self::USED == 0 {
            None
        } else {
            Some(Self(word))
        }
    }

    const fn port(self) -> u8 {
        (self.0 & 0x3F) as u8
    }

    const fn rights(self) -> Rights {
        Rights(((self.0 >> 6) & 3) as u8)
    }

    const fn generation(self) -> u16 {
        (self.0 >> 8) as u16
    }

    const fn with_generation(self, generation: u16) -> Self {
        Self(self.0 & !0x00FF_FF00 | (generation as u32) << 8)
    }
}

/// Where a call goes: switch to `slot` and run `entry`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enter {
    /// The server's slot.
    pub slot: u8,
    /// Its entry point.
    pub entry: u32,
    /// Calls the thread is now in (1 for a first-level call).
    pub depth: usize,
}

/// The kernel's IPC tables.
pub struct Ipc {
    ports: [Port; MAX_PORTS],
    tables: [Table; MAX_SLOTS],
    /// Per thread number (1 to [`MAX_THREADS`]): the calls it is in.
    depth: [u8; MAX_THREADS + 1],
}

impl Default for Ipc {
    fn default() -> Self {
        Self::new()
    }
}

impl Ipc {
    /// Empty tables.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ports: [NO_PORT; MAX_PORTS],
            tables: [EMPTY_TABLE; MAX_SLOTS],
            depth: [0; MAX_THREADS + 1],
        }
    }

    /// A process slot's table index: CE's slots, then the native ones.
    fn slot_ok(slot: u8) -> Result<usize, Error> {
        if (FIRST_SLOT..=LAST_SLOT).contains(&slot) {
            Ok(usize::from(slot - FIRST_SLOT))
        } else if (FIRST_NATIVE_SLOT..=LAST_NATIVE_SLOT).contains(&slot) {
            Ok(usize::from(
                slot - FIRST_NATIVE_SLOT + LAST_SLOT - FIRST_SLOT + 1,
            ))
        } else {
            Err(Error::Range)
        }
    }

    /// A thread number's index (1 to [`MAX_THREADS`]; 0 is no thread).
    fn thread_ok(tid: u8) -> Result<usize, Error> {
        let t = usize::from(tid);
        if t == 0 || t > MAX_THREADS {
            Err(Error::Range)
        } else {
            Ok(t)
        }
    }

    /// Opens a port served by `server` at `entry`; the server gets a handle
    /// to it with every right.
    pub fn open(
        &mut self,
        hp: &mut (impl HandlePages + ?Sized),
        server: u8,
        entry: u32,
    ) -> Result<(PortId, Handle), Error> {
        Self::slot_ok(server)?;
        // A free entry, or a closed port's: reused with the next generation,
        // so handles to the old port can never reach the new one.
        let i = self
            .ports
            .iter()
            .position(|p| p.open == 0)
            .ok_or(Error::Full)?;
        let generation = self.ports[i].generation.wrapping_add(1);
        self.ports[i] = Port {
            server,
            entry,
            generation,
            open: 1,
        };
        let port = PortId(i as u8);
        match self.install(hp, server, port, Rights::ALL) {
            Ok(h) => Ok((port, h)),
            Err(e) => {
                self.ports[i].open = 0;
                Err(e)
            }
        }
    }

    /// Puts a handle to `port` in `slot`'s table (the kernel's own act, as
    /// when the service manager starts a process). The table grows into a
    /// new page when its inline handles and pages are all taken, within the
    /// quota.
    pub fn install(
        &mut self,
        hp: &mut (impl HandlePages + ?Sized),
        slot: u8,
        port: PortId,
        rights: Rights,
    ) -> Result<Handle, Error> {
        let s = Self::slot_ok(slot)?;
        let p = self
            .ports
            .get(usize::from(port.0))
            .filter(|p| p.open == 1)
            .ok_or(Error::Closed)?;
        let generation = p.generation;
        let table = &mut self.tables[s];
        let i = table.free_slot(hp)?;
        table.set(hp, i, Entry::new(port.0, rights, generation).0);
        Ok(Handle(i as u16))
    }

    fn entry(
        &self,
        hp: &mut (impl HandlePages + ?Sized),
        slot: u8,
        h: Handle,
    ) -> Result<Entry, Error> {
        let s = Self::slot_ok(slot)?;
        self.tables[s]
            .get(hp, usize::from(h.0))
            .and_then(Entry::decode)
            .ok_or(Error::BadHandle)
    }

    /// Hands `from`'s handle on to `to`, with `rights` (which must be
    /// among the handle's own; `GRANT` needed).
    pub fn grant(
        &mut self,
        hp: &mut (impl HandlePages + ?Sized),
        from: u8,
        h: Handle,
        to: u8,
        rights: Rights,
    ) -> Result<Handle, Error> {
        let e = self.entry(hp, from, h)?;
        if !e.rights().contains(Rights::GRANT) || !e.rights().contains(rights) {
            return Err(Error::NoRight);
        }
        self.live(e)?;
        self.install(hp, to, PortId(e.port()), rights)
    }

    /// The port a handle names, if it is still the same one.
    fn live(&self, e: Entry) -> Result<Port, Error> {
        let p = self.ports[usize::from(e.port())];
        if p.open == 0 {
            return Err(Error::Closed);
        }
        if p.generation != e.generation() {
            return Err(Error::Stale);
        }
        Ok(p)
    }

    /// Thread `tid` (running in `slot`) calls through `h`: where it goes.
    pub fn call(
        &mut self,
        hp: &mut (impl HandlePages + ?Sized),
        tid: u8,
        slot: u8,
        h: Handle,
    ) -> Result<Enter, Error> {
        let t = Self::thread_ok(tid)?;
        let e = self.entry(hp, slot, h)?;
        if !e.rights().contains(Rights::CALL) {
            return Err(Error::NoRight);
        }
        let p = self.live(e)?;
        let d = usize::from(self.depth[t]);
        if d >= MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        self.depth[t] += 1;
        Ok(Enter {
            slot: p.server,
            entry: p.entry,
            depth: d + 1,
        })
    }

    /// Thread `tid` returns from the call it is in.
    pub fn ret(&mut self, tid: u8) -> Result<(), Error> {
        let t = Self::thread_ok(tid)?;
        let d = self.depth[t];
        if d == 0 {
            return Err(Error::NotInCall);
        }
        self.depth[t] = d - 1;
        Ok(())
    }

    /// The server faulted while `tid` was in it: unwind one call (the
    /// caller gets an error); false if the thread wasn't in a call (the
    /// fault is its own process's).
    pub fn fault(&mut self, tid: u8) -> bool {
        self.ret(tid).is_ok()
    }

    /// Calls `tid` is in.
    #[must_use]
    pub fn depth(&self, tid: u8) -> usize {
        self.depth
            .get(usize::from(tid))
            .map_or(0, |&d| usize::from(d))
    }

    /// The server was restarted: the port stays, but handles taken before
    /// now are stale. The server's own handles are renewed, and so are its
    /// supervisor's (`supervisor`), which hands out fresh ones.
    pub fn restart(
        &mut self,
        hp: &mut (impl HandlePages + ?Sized),
        port: PortId,
        supervisor: Option<u8>,
    ) -> Result<(), Error> {
        let p = self
            .ports
            .get_mut(usize::from(port.0))
            .filter(|p| p.server != 0)
            .ok_or(Error::Closed)?;
        let server = p.server;
        let sup = supervisor.map(Self::slot_ok).transpose()?;
        let s = Self::slot_ok(server)?;
        p.generation = p.generation.wrapping_add(1);
        let generation = p.generation;
        for t in core::iter::once(s).chain(sup) {
            self.tables[t].update(hp, |w| match Entry::decode(w) {
                Some(e) if e.port() == port.0 => e.with_generation(generation).0,
                _ => w,
            });
        }
        Ok(())
    }

    /// Handles in `slot`'s table (0 for a slot out of range).
    pub fn handle_count(&mut self, hp: &mut (impl HandlePages + ?Sized), slot: u8) -> usize {
        let Ok(s) = Self::slot_ok(slot) else {
            return 0;
        };
        let mut n = 0;
        self.tables[s].update(hp, |w| {
            n += usize::from(Entry::decode(w).is_some());
            w
        });
        n
    }

    /// Pages `slot`'s handle table holds (0 for a slot out of range).
    #[must_use]
    pub fn handle_pages(&self, slot: u8) -> usize {
        Self::slot_ok(slot).map_or(0, |s| (self.tables[s].reach() - INLINE_HANDLES) / PER_PAGE)
    }

    /// Open ports served by `slot`.
    #[must_use]
    pub fn ports_of(&self, slot: u8) -> usize {
        self.ports
            .iter()
            .filter(|p| p.open == 1 && p.server == slot)
            .count()
    }

    /// A process exited: its ports close (callers get `Closed`) and its
    /// handle table empties, its pages going back.
    pub fn exit(&mut self, hp: &mut (impl HandlePages + ?Sized), slot: u8) -> Result<(), Error> {
        let s = Self::slot_ok(slot)?;
        for p in &mut self.ports {
            if p.server == slot {
                p.open = 0;
            }
        }
        self.tables[s].clear(hp);
        Ok(())
    }

    /// Drops a handle from `slot`'s table.
    pub fn close(
        &mut self,
        hp: &mut (impl HandlePages + ?Sized),
        slot: u8,
        h: Handle,
    ) -> Result<(), Error> {
        let s = Self::slot_ok(slot)?;
        let i = usize::from(h.0);
        let w = self.tables[s].get(hp, i).ok_or(Error::BadHandle)?;
        if Entry::decode(w).is_none() {
            return Err(Error::BadHandle);
        }
        self.tables[s].set(hp, i, Entry::EMPTY);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
