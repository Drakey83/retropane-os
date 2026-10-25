//! The session's object handles [MTP 3.4]: assigned as the PC lists
//! folders, unique across stores, never reused within the session (a
//! deleted object's slot stays dead), kept across moves. A slot is 16
//! bytes; a hash index (two `u32` per slot) finds a location's handle
//! without a scan. Both live in memory the owner gives (static in the
//! loader, never its stack).

use crate::device::Loc;

/// One handle's object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    /// Its location on its store.
    pub loc: Loc,
    /// The parent folder's handle (0: the store's root).
    pub parent: u32,
    /// Its store number; [`DEAD`] once deleted.
    pub store: u8,
    /// A folder.
    pub folder: bool,
}

/// A deleted object's slot: its handle never comes back.
pub const DEAD: u8 = 0xFF;

/// Why a handle couldn't be assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// The handle table.
pub struct Handles<'a> {
    slots: &'a mut [Slot],
    /// Open addressing over (store, location): slot index + 1, 0 empty.
    index: &'a mut [u32],
    used: usize,
}

fn hash(store: u8, loc: Loc) -> u64 {
    // SplitMix64's finaliser: locations are cluster numbers and offsets,
    // close together; this spreads them.
    let mut z = loc ^ (u64::from(store) << 56) ^ 0x9E37_79B9_7F4A_7C15;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl<'a> Handles<'a> {
    /// A table over `slots`, indexed by `index` (at least twice as long,
    /// so the index never fills).
    ///
    /// # Panics
    /// If `index` is shorter than twice `slots`.
    pub fn new(slots: &'a mut [Slot], index: &'a mut [u32]) -> Self {
        assert!(index.len() >= 2 * slots.len(), "handle index too small");
        let mut h = Self {
            slots,
            index,
            used: 0,
        };
        h.clear();
        h
    }

    /// Forgets every handle (a new session).
    pub fn clear(&mut self) {
        self.index.fill(0);
        self.used = 0;
    }

    /// Handles assigned in this session (dead ones included).
    #[must_use]
    pub fn len(&self) -> usize {
        self.used
    }

    /// Whether none is assigned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// How many the table holds.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    fn probe(&self, store: u8, loc: Loc) -> (usize, Option<usize>) {
        let n = self.index.len();
        let mut i = (hash(store, loc) % n as u64) as usize;
        loop {
            match self.index[i] {
                0 => return (i, None),
                k => {
                    let s = &self.slots[k as usize - 1];
                    if s.store == store && s.loc == loc {
                        return (i, Some(k as usize - 1));
                    }
                }
            }
            i = (i + 1) % n;
        }
    }

    /// The live handle of `loc` on `store`, if one was assigned.
    #[must_use]
    pub fn find(&self, store: u8, loc: Loc) -> Option<u32> {
        self.probe(store, loc).1.map(|k| k as u32 + 1)
    }

    /// The handle of `loc`, assigned now if it has none.
    ///
    /// # Errors
    /// [`Full`] when the table is.
    pub fn get_or_add(
        &mut self,
        store: u8,
        loc: Loc,
        parent: u32,
        folder: bool,
    ) -> Result<u32, Full> {
        let (i, found) = self.probe(store, loc);
        if let Some(k) = found {
            let s = &mut self.slots[k];
            s.parent = parent;
            s.folder = folder;
            return Ok(k as u32 + 1);
        }
        if self.used == self.slots.len() {
            return Err(Full);
        }
        let k = self.used;
        self.slots[k] = Slot {
            loc,
            parent,
            store,
            folder,
        };
        self.used += 1;
        self.index[i] = k as u32 + 1;
        Ok(k as u32 + 1)
    }

    /// A live handle's slot.
    #[must_use]
    pub fn get(&self, handle: u32) -> Option<Slot> {
        let k = (handle as usize).checked_sub(1)?;
        let s = *self.slots.get(k).filter(|_| k < self.used)?;
        (s.store != DEAD).then_some(s)
    }

    /// The slot at table position `k` (dead or alive), for walks.
    #[must_use]
    pub fn at(&self, k: usize) -> Option<Slot> {
        (k < self.used).then(|| self.slots[k])
    }

    fn unindex(&mut self, k: usize) {
        let s = self.slots[k];
        let (mut i, found) = self.probe(s.store, s.loc);
        if found != Some(k) {
            return;
        }
        // Backward-shift deletion keeps every probe chain whole.
        let n = self.index.len();
        self.index[i] = 0;
        let mut j = (i + 1) % n;
        while self.index[j] != 0 {
            let m = self.index[j] as usize - 1;
            let t = &self.slots[m];
            let home = (hash(t.store, t.loc) % n as u64) as usize;
            let between = if i <= j {
                i < home && home <= j
            } else {
                i < home || home <= j
            };
            if !between {
                self.index[i] = self.index[j];
                self.index[j] = 0;
                i = j;
            }
            j = (j + 1) % n;
        }
    }

    /// The object is gone: its handle never comes back this session.
    pub fn kill(&mut self, handle: u32) {
        if let Some(k) = (handle as usize)
            .checked_sub(1)
            .filter(|&k| k < self.used && self.slots[k].store != DEAD)
        {
            self.unindex(k);
            self.slots[k].store = DEAD;
        }
    }

    /// The object moved (renamed, moved, committed): same handle, new place.
    pub fn relocate(&mut self, handle: u32, store: u8, loc: Loc, parent: u32) {
        let Some(k) = (handle as usize).checked_sub(1).filter(|&k| k < self.used) else {
            return;
        };
        if self.slots[k].store == DEAD {
            return;
        }
        self.unindex(k);
        // Another live handle on the new place (it was listed under its old
        // name meanwhile, impossible in practice) gives way.
        if let (_, Some(other)) = self.probe(store, loc) {
            self.unindex(other);
            self.slots[other].store = DEAD;
        }
        self.slots[k].store = store;
        self.slots[k].loc = loc;
        self.slots[k].parent = parent;
        let (i, _) = self.probe(store, loc);
        self.index[i] = k as u32 + 1;
    }

    /// Kills every handle of store `s` (its card went).
    pub fn kill_store(&mut self, s: u8) {
        for k in 0..self.used {
            if self.slots[k].store == s {
                self.unindex(k);
                self.slots[k].store = DEAD;
            }
        }
    }

    /// Whether `handle` lies under folder `top` (or is it), within `depth`
    /// levels (0: only `top`); `top` 0 means a store's root (`store`).
    #[must_use]
    pub fn under(&self, handle: u32, store: u8, top: u32, depth: u32) -> Option<u32> {
        let mut h = handle;
        let mut level = 0u32;
        loop {
            if h == top {
                return (level <= depth).then_some(level);
            }
            let s = self.get(h)?;
            if s.store != store {
                return None;
            }
            h = s.parent;
            level += 1;
            if level > depth.saturating_add(1) || level > 4096 {
                return None;
            }
        }
    }
}
