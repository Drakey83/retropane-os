//! Service calls between processes (ADR-0016): the rules, separate from the
//! context switch that carries them out.
//!
//! A server process opens a **port** (an entry point in its slot). Other
//! processes reach it only through a **handle** in their own handle table,
//! with **rights**: `CALL` to call it, `GRANT` to hand it on (only ever with
//! the same rights or fewer). A call **migrates** the calling thread into
//! the server's slot: [`Ipc::call`] checks everything and pushes a frame
//! (where to come back to), [`Ipc::ret`] pops it; a server that faults
//! mid-call unwinds to its caller ([`Ipc::fault`]), which gets an error.
//! Restarting a server bumps its port's **generation**, so handles taken
//! before the restart fail cleanly instead of reaching the new instance.
//!
//! Security (docs/security.md): a handle is an index into the caller's own
//! table, so a process can't name another's handles; forged or stale
//! numbers fail; rights can't grow; nesting is bounded; every table is a
//! fixed size, so nothing an attacker does grows kernel memory.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Ports in the system.
pub const MAX_PORTS: usize = 64;
/// Handles per process.
pub const MAX_HANDLES: usize = 32;
/// The first and last process slots (`rp_kernel_mm::layout`, CE5's
/// layout: slot 0 is the running process's alias, slot 1 shared code, and
/// above slot 32 lies the shared-file area, so 31 processes). The kernel
/// checks at build time that these match the layout.
pub const FIRST_SLOT: u8 = 2;
/// See [`FIRST_SLOT`].
pub const LAST_SLOT: u8 = 32;
/// Process slots, so handle tables: one per process, none for slots that
/// can't hold one (4 KiB in all, not the 8 KiB of a table per FCSE PID).
pub const MAX_SLOTS: usize = (LAST_SLOT - FIRST_SLOT + 1) as usize;
/// Threads (as the scheduler numbers them).
pub const MAX_THREADS: usize = 128;
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
pub struct Handle(pub u8);

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

#[derive(Clone, Copy)]
struct Port {
    server: u8,
    entry: u32,
    generation: u16,
    /// 1 while open. A byte, not a `bool`: a `bool` gives `Option<Port>` a
    /// niche, so `None` would be the byte 2 and every `Ipc` static would be
    /// stored in the image instead of zeroed at start (13.7 KB, measured).
    open: u8,
}

/// A handle table entry, stored as one word: 0 when empty, else bit 31
/// set, the generation in bits 8..24, the rights in bits 6..8 and the port
/// (64 ports) in bits 0..6. No `Option` and no padding: the system's handle
/// tables are exactly 8 KiB, all zero at start (so in `.bss`, not the image).
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

/// Where a thread goes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Return {
    /// The caller's slot.
    pub slot: u8,
}

/// The kernel's IPC tables.
pub struct Ipc {
    ports: [Option<Port>; MAX_PORTS],
    tables: [[u32; MAX_HANDLES]; MAX_SLOTS],
    /// Per thread: the caller slots of the calls it is in.
    frames: [[u8; MAX_DEPTH]; MAX_THREADS],
    depth: [u8; MAX_THREADS],
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
            ports: [None; MAX_PORTS],
            tables: [[Entry::EMPTY; MAX_HANDLES]; MAX_SLOTS],
            frames: [[0; MAX_DEPTH]; MAX_THREADS],
            depth: [0; MAX_THREADS],
        }
    }

    /// A process slot's table index.
    fn slot_ok(slot: u8) -> Result<usize, Error> {
        if (FIRST_SLOT..=LAST_SLOT).contains(&slot) {
            Ok(usize::from(slot - FIRST_SLOT))
        } else {
            Err(Error::Range)
        }
    }

    /// Opens a port served by `server` at `entry`; the server gets a handle
    /// to it with every right.
    pub fn open(&mut self, server: u8, entry: u32) -> Result<(PortId, Handle), Error> {
        Self::slot_ok(server)?;
        // A free entry, or a closed port's: reused with the next generation,
        // so handles to the old port can never reach the new one.
        let i = self
            .ports
            .iter()
            .position(|p| p.is_none_or(|p| p.open == 0))
            .ok_or(Error::Full)?;
        let generation = self.ports[i].map_or(0, |p| p.generation.wrapping_add(1));
        self.ports[i] = Some(Port {
            server,
            entry,
            generation,
            open: 1,
        });
        let port = PortId(i as u8);
        match self.install(server, port, Rights::ALL) {
            Ok(h) => Ok((port, h)),
            Err(e) => {
                self.ports[i] = None;
                Err(e)
            }
        }
    }

    /// Puts a handle to `port` in `slot`'s table (the kernel's own act, as
    /// when the service manager starts a process).
    pub fn install(&mut self, slot: u8, port: PortId, rights: Rights) -> Result<Handle, Error> {
        let s = Self::slot_ok(slot)?;
        let p = self
            .ports
            .get(usize::from(port.0))
            .copied()
            .flatten()
            .ok_or(Error::Closed)?;
        let table = &mut self.tables[s];
        let i = table
            .iter()
            .position(|&w| w == Entry::EMPTY)
            .ok_or(Error::Full)?;
        table[i] = Entry::new(port.0, rights, p.generation).0;
        Ok(Handle(i as u8))
    }

    fn entry(&self, slot: u8, h: Handle) -> Result<Entry, Error> {
        let s = Self::slot_ok(slot)?;
        self.tables[s]
            .get(usize::from(h.0))
            .and_then(|&w| Entry::decode(w))
            .ok_or(Error::BadHandle)
    }

    /// Hands `from`'s handle on to `to`, with `rights` (which must be
    /// among the handle's own; `GRANT` needed).
    pub fn grant(&mut self, from: u8, h: Handle, to: u8, rights: Rights) -> Result<Handle, Error> {
        let e = self.entry(from, h)?;
        if !e.rights().contains(Rights::GRANT) || !e.rights().contains(rights) {
            return Err(Error::NoRight);
        }
        self.live(e)?;
        self.install(to, PortId(e.port()), rights)
    }

    /// The port a handle names, if it is still the same one.
    fn live(&self, e: Entry) -> Result<Port, Error> {
        let p = self.ports[usize::from(e.port())].ok_or(Error::Closed)?;
        if p.open == 0 {
            return Err(Error::Closed);
        }
        if p.generation != e.generation() {
            return Err(Error::Stale);
        }
        Ok(p)
    }

    /// Thread `tid` (running in `slot`) calls through `h`: where it goes.
    pub fn call(&mut self, tid: u8, slot: u8, h: Handle) -> Result<Enter, Error> {
        let t = usize::from(tid);
        if t >= MAX_THREADS {
            return Err(Error::Range);
        }
        let e = self.entry(slot, h)?;
        if !e.rights().contains(Rights::CALL) {
            return Err(Error::NoRight);
        }
        let p = self.live(e)?;
        let d = usize::from(self.depth[t]);
        if d >= MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        self.frames[t][d] = slot;
        self.depth[t] += 1;
        Ok(Enter {
            slot: p.server,
            entry: p.entry,
            depth: d + 1,
        })
    }

    /// Thread `tid` returns from the call it is in: where it goes back to.
    pub fn ret(&mut self, tid: u8) -> Result<Return, Error> {
        let t = usize::from(tid);
        if t >= MAX_THREADS {
            return Err(Error::Range);
        }
        let d = self.depth[t];
        if d == 0 {
            return Err(Error::NotInCall);
        }
        self.depth[t] = d - 1;
        Ok(Return {
            slot: self.frames[t][usize::from(d - 1)],
        })
    }

    /// The server faulted while `tid` was in it: unwind one call (the
    /// caller gets an error); `None` if the thread wasn't in a call (the
    /// fault is its own process's).
    pub fn fault(&mut self, tid: u8) -> Option<Return> {
        self.ret(tid).ok()
    }

    /// Calls `tid` is in.
    #[must_use]
    pub fn depth(&self, tid: u8) -> usize {
        self.depth
            .get(usize::from(tid))
            .map_or(0, |&d| usize::from(d))
    }

    /// The server was restarted: the port stays, but handles taken before
    /// now are stale (the server's own handle is renewed).
    pub fn restart(&mut self, port: PortId) -> Result<(), Error> {
        let p = self
            .ports
            .get_mut(usize::from(port.0))
            .and_then(Option::as_mut)
            .ok_or(Error::Closed)?;
        p.generation = p.generation.wrapping_add(1);
        let (server, generation) = (p.server, p.generation);
        let s = Self::slot_ok(server)?;
        for w in &mut self.tables[s] {
            if let Some(e) = Entry::decode(*w).filter(|e| e.port() == port.0) {
                *w = e.with_generation(generation).0;
            }
        }
        Ok(())
    }

    /// A process exited: its ports close (callers get `Closed`) and its
    /// handle table empties.
    pub fn exit(&mut self, slot: u8) -> Result<(), Error> {
        let s = Self::slot_ok(slot)?;
        for p in self.ports.iter_mut().flatten() {
            if p.server == slot {
                p.open = 0;
            }
        }
        self.tables[s] = [Entry::EMPTY; MAX_HANDLES];
        Ok(())
    }

    /// Drops a handle from `slot`'s table.
    pub fn close(&mut self, slot: u8, h: Handle) -> Result<(), Error> {
        let s = Self::slot_ok(slot)?;
        let w = self.tables[s]
            .get_mut(usize::from(h.0))
            .ok_or(Error::BadHandle)?;
        if Entry::decode(core::mem::take(w)).is_none() {
            return Err(Error::BadHandle);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
